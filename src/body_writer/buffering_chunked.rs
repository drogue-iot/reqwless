use embedded_io::{Error as _, ErrorType};
use embedded_io_async::Write;

use super::chunked::write_chunked_header;

const EMPTY_CHUNK: &[u8; 5] = b"0\r\n\r\n";
const NEWLINE: &[u8; 2] = b"\r\n";

/// A body writer that buffers internally and emits chunks as expected by the
/// `Transfer-Encoding: chunked` header specification.
///
/// Each emittted chunk has a header that specifies the size of the chunk,
/// and the last chunk has size equal to zero, indicating the end of the request.
///
/// The writer can be seeded with a buffer that is already pre-written. This is
/// typical if for example the request header is already written to the buffer.
/// The writer will in this case start appending a chunk to the end of the pre-written
/// buffer data leaving the pre-written data as-is.
///
/// To minimize the number of write calls to the underlying connection the writer
/// works by pre-allocating the chunk header in the buffer. The written body data is
/// then appended after this pre-allocated header. Depending on the number of bytes
/// actually written to the current chunk before the writer is terminated (indicating
/// the end of the request body), the pre-allocated header may be too large. If this
/// is the case, then the chunk payload is moved into the pre-allocated header region
/// such that the header and payload can be written to the underlying connection in
/// a single write.
///
pub struct BufferingChunkedBodyWriter<'a, C: Write> {
    conn: C,
    buf: &'a mut [u8],
    /// The position where the allocated chunk header starts
    header_pos: usize,
    /// The size of the allocated header (the final header may be smaller)
    /// This may be 0 if the pre-written bytes in `buf` is too large to fit a header.
    allocated_header: usize,
    /// The position of the data in the chunk
    pos: usize,
    terminated: bool,
}

impl<'a, C> BufferingChunkedBodyWriter<'a, C>
where
    C: Write,
{
    pub fn new_with_data(conn: C, buf: &'a mut [u8], written: usize) -> Self {
        assert!(written <= buf.len());
        let allocated_header = get_max_chunk_header_size(buf.len() - written);
        assert!(buf.len() > allocated_header + NEWLINE.len()); // There must be space for the chunk header and footer
        Self {
            conn,
            buf,
            header_pos: written,
            pos: written + allocated_header,
            allocated_header,
            terminated: false,
        }
    }

    /// Terminate the request body by writing an empty chunk
    pub async fn terminate(&mut self) -> Result<(), C::Error> {
        assert!(!self.terminated);

        if self.pos > self.header_pos + self.allocated_header {
            // There are bytes written in the current chunk
            self.finish_current_chunk();
        }

        if self.header_pos + EMPTY_CHUNK.len() > self.buf.len() {
            // There is not enough space to fit the empty chunk in the buffer
            self.emit_buffered().await?;
        }

        self.buf[self.header_pos..self.header_pos + EMPTY_CHUNK.len()].copy_from_slice(EMPTY_CHUNK);
        self.header_pos += EMPTY_CHUNK.len();
        self.allocated_header = 0;
        self.pos = self.header_pos + self.allocated_header;
        self.emit_buffered().await?;
        self.terminated = true;
        Ok(())
    }

    /// Append data to the current chunk and return the number of bytes appended.
    /// This returns 0 if there is no current chunk to append to.
    fn append_current_chunk(&mut self, buf: &[u8]) -> usize {
        let buffered = usize::min(buf.len(), self.buf.len().saturating_sub(NEWLINE.len() + self.pos));
        if buffered > 0 {
            self.buf[self.pos..self.pos + buffered].copy_from_slice(&buf[..buffered]);
            self.pos += buffered;
        }
        buffered
    }

    /// Finish the current chunk by writing the header
    fn finish_current_chunk(&mut self) {
        // Write the header in the allocated position position
        let chunk_len = self.pos - self.header_pos - self.allocated_header;
        let header_buf = &mut self.buf[self.header_pos..self.header_pos + self.allocated_header];
        let header_len = write_chunked_header(header_buf, chunk_len);

        // Move the payload if the header length was not as large as it could possibly be
        let spacing = self.allocated_header - header_len;
        if spacing > 0 {
            self.buf.copy_within(
                self.header_pos + self.allocated_header..self.pos,
                self.header_pos + header_len,
            );
            self.pos -= spacing
        }

        // Write newline footer after chunk payload
        self.buf[self.pos..self.pos + NEWLINE.len()].copy_from_slice(NEWLINE);
        self.pos += 2;

        self.header_pos = self.pos;
        self.allocated_header = get_max_chunk_header_size(self.buf.len() - self.header_pos);
        self.pos = self.header_pos + self.allocated_header;
    }

    fn current_chunk_is_full(&self) -> bool {
        self.pos + NEWLINE.len() == self.buf.len()
    }

    async fn emit_buffered(&mut self) -> Result<(), C::Error> {
        self.conn.write_all(&self.buf[..self.header_pos]).await?;
        self.header_pos = 0;
        self.allocated_header = get_max_chunk_header_size(self.buf.len());
        self.pos = self.allocated_header;
        Ok(())
    }
}

impl<C> ErrorType for BufferingChunkedBodyWriter<'_, C>
where
    C: Write,
{
    type Error = embedded_io::ErrorKind;
}

impl<C> Write for BufferingChunkedBodyWriter<'_, C>
where
    C: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut written = self.append_current_chunk(buf);
        if written == 0 {
            // Unable to append any data to the buffer
            // This can happen if the writer was pre-loaded with data
            self.emit_buffered().await.map_err(|e| e.kind())?;
            written = self.append_current_chunk(buf);
        }
        if self.current_chunk_is_full() {
            self.finish_current_chunk();
            self.emit_buffered().await.map_err(|e| e.kind())?;
        }
        Ok(written)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        if self.pos > self.header_pos + self.allocated_header {
            // There are bytes written in the current chunk
            self.finish_current_chunk();
            self.emit_buffered().await.map_err(|e| e.kind())?;
        } else if self.header_pos > 0 {
            // There are pre-written bytes in the buffer but no current chunk
            // (the number of pre-written was so large that the space for a header could not be allocated)
            self.emit_buffered().await.map_err(|e| e.kind())?;
        }
        self.conn.flush().await.map_err(|e| e.kind())
    }
}

/// Get the number of hex characters for a number.
/// E.g. 0x0 => 1, 0x0F => 1, 0x10 => 2, 0x1234 => 4.
const fn get_num_hex_chars(number: usize) -> usize {
    if number == 0 {
        1
    } else {
        (usize::BITS - number.leading_zeros()).div_ceil(4) as usize
    }
}

const fn get_max_chunk_header_size(buffer_size: usize) -> usize {
    if let Some(hex_chars_and_payload_size) = buffer_size.checked_sub(2 * NEWLINE.len()) {
        get_num_hex_chars(hex_chars_and_payload_size) + NEWLINE.len()
    } else {
        // Not enough space in buffer to fit a header + footer
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_get_hex_chars() {
        assert_eq!(1, get_num_hex_chars(0));
        assert_eq!(1, get_num_hex_chars(1));
        assert_eq!(1, get_num_hex_chars(0xF));
        assert_eq!(2, get_num_hex_chars(0x10));
        assert_eq!(2, get_num_hex_chars(0xFF));
        assert_eq!(3, get_num_hex_chars(0x100));
    }

    #[test]
    fn can_get_max_chunk_header_size() {
        assert_eq!(0, get_max_chunk_header_size(0));
        assert_eq!(0, get_max_chunk_header_size(1));
        assert_eq!(0, get_max_chunk_header_size(2));
        assert_eq!(0, get_max_chunk_header_size(3));
        assert_eq!(3, get_max_chunk_header_size(0x00 + 2 + 2));
        assert_eq!(3, get_max_chunk_header_size(0x01 + 2 + 2));
        assert_eq!(3, get_max_chunk_header_size(0x0F + 2 + 2));
        assert_eq!(4, get_max_chunk_header_size(0x10 + 2 + 2));
        assert_eq!(4, get_max_chunk_header_size(0x11 + 2 + 2));
        assert_eq!(4, get_max_chunk_header_size(0x12 + 2 + 2));
    }

    #[tokio::test]
    async fn preserves_already_written_bytes_in_the_buffer_without_any_chunks() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 1024];
        buf[..5].copy_from_slice(b"HELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 5);
        writer.terminate().await.unwrap();

        // Then
        assert_eq!(b"HELLO0\r\n\r\n", conn.as_slice());
    }

    #[tokio::test]
    async fn preserves_already_written_bytes_in_the_buffer_with_chunks() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 1024];
        buf[..5].copy_from_slice(b"HELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 5);
        writer.write_all(b"BODY").await.unwrap();
        writer.terminate().await.unwrap();

        // Then
        assert_eq!(b"HELLO4\r\nBODY\r\n0\r\n\r\n", conn.as_slice());
    }

    #[tokio::test]
    async fn write_when_entire_buffer_is_prewritten() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 10];
        buf.copy_from_slice(b"HELLOHELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 10);
        writer.write_all(b"BODY").await.unwrap(); // Cannot fit
        writer.terminate().await.unwrap();

        // Then
        print!("{:?}", conn.as_slice());
        assert_eq!(b"HELLOHELLO4\r\nBODY\r\n0\r\n\r\n", conn.as_slice());
    }

    #[tokio::test]
    async fn flush_empty_body_when_entire_buffer_is_prewritten() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 10];
        buf.copy_from_slice(b"HELLOHELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 10);
        writer.flush().await.unwrap();

        // Then
        print!("{:?}", conn.as_slice());
        assert_eq!(b"HELLOHELLO", conn.as_slice());
    }

    #[tokio::test]
    async fn terminate_empty_body_when_entire_buffer_is_prewritten() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 10];
        buf.copy_from_slice(b"HELLOHELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 10);
        writer.terminate().await.unwrap();

        // Then
        print!("{:?}", conn.as_slice());
        assert_eq!(b"HELLOHELLO0\r\n\r\n", conn.as_slice());
    }

    #[tokio::test]
    async fn flush_when_entire_buffer_is_nearly_prewritten() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 11];
        buf[..10].copy_from_slice(b"HELLOHELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 10);
        writer.flush().await.unwrap();

        // Then
        print!("{:?}", conn.as_slice());
        assert_eq!(b"HELLOHELLO", conn.as_slice());
    }

    #[tokio::test]
    async fn flushes_already_written_bytes_if_first_cannot_fit() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 10];
        buf[..5].copy_from_slice(b"HELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 5);
        writer.write_all(b"BODY").await.unwrap(); // Cannot fit
        writer.terminate().await.unwrap(); // Can fit

        // Then
        assert_eq!(b"HELLO4\r\nBODY\r\n0\r\n\r\n", conn.as_slice());
    }

    #[tokio::test]
    async fn written_bytes_fit_exactly() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 14];
        buf[..5].copy_from_slice(b"HELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 5);
        writer.write_all(b"BODY").await.unwrap(); // Can fit exactly
        writer.write_all(b"BODY").await.unwrap(); // Can fit
        writer.terminate().await.unwrap(); // Can fit

        // Then
        assert_eq!(b"HELLO4\r\nBODY\r\n4\r\nBODY\r\n0\r\n\r\n", conn.as_slice());
    }

    #[tokio::test]
    async fn current_chunk_is_emitted_on_termination_before_empty_chunk_is_emitted() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 14];
        buf[..5].copy_from_slice(b"HELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 5);
        writer.write_all(b"BOD").await.unwrap(); // Can fit
        writer.terminate().await.unwrap(); // Cannot fit

        // Then
        assert_eq!(b"HELLO3\r\nBOD\r\n0\r\n\r\n", conn.as_slice());
    }

    #[tokio::test]
    async fn write_emits_chunks() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 12];
        buf[..5].copy_from_slice(b"HELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 5);
        writer.write_all(b"BODY").await.unwrap(); // Only "BO" can fit first, then "DY" is written in a different chunk
        writer.terminate().await.unwrap();

        // Then
        assert_eq!(b"HELLO2\r\nBO\r\n2\r\nDY\r\n0\r\n\r\n", conn.as_slice());
    }
}
