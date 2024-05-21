use core::mem::size_of;

use embedded_io::{Error as _, ErrorType};
use embedded_io_async::Write;

const NEWLINE: &[u8; 2] = b"\r\n";
const EMPTY_CHUNK: &[u8; 5] = b"0\r\n\r\n";

pub struct FixedBodyWriter<C: Write>(C, usize);

impl<C> FixedBodyWriter<C>
where
    C: Write,
{
    pub fn new(conn: C) -> Self {
        Self(conn, 0)
    }

    pub fn written(&self) -> usize {
        self.1
    }
}

impl<C> ErrorType for FixedBodyWriter<C>
where
    C: Write,
{
    type Error = C::Error;
}

impl<C> Write for FixedBodyWriter<C>
where
    C: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let written = self.0.write(buf).await?;
        self.1 += written;
        Ok(written)
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        self.0.write_all(buf).await?;
        self.1 += buf.len();
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

pub struct ChunkedBodyWriter<C: Write>(C);

impl<C> ChunkedBodyWriter<C>
where
    C: Write,
{
    pub fn new(conn: C) -> Self {
        Self(conn)
    }

    /// Terminate the request body by writing an empty chunk
    pub async fn terminate(&mut self) -> Result<(), C::Error> {
        self.0.write_all(EMPTY_CHUNK).await
    }
}

impl<C> ErrorType for ChunkedBodyWriter<C>
where
    C: Write,
{
    type Error = embedded_io::ErrorKind;
}

impl<C> Write for ChunkedBodyWriter<C>
where
    C: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.write_all(buf).await.map_err(|e| e.kind())?;
        Ok(buf.len())
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        let len = buf.len();

        // Do not write an empty chunk as that will terminate the body
        // Use `ChunkedBodyWriter.write_empty_chunk` instead if this is intended
        if len == 0 {
            return Ok(());
        }

        // Write chunk header
        let mut header_buf = [0; 2 * size_of::<usize>() + 2];
        let header_len = write_chunked_header(&mut header_buf, len);
        self.0
            .write_all(&header_buf[..header_len])
            .await
            .map_err(|e| e.kind())?;

        // Write chunk
        self.0.write_all(buf).await.map_err(|e| e.kind())?;

        // Write newline footer
        self.0.write_all(NEWLINE).await.map_err(|e| e.kind())?;
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await.map_err(|e| e.kind())
    }
}

pub struct BufferingChunkedBodyWriter<'a, C: Write> {
    conn: C,
    buf: &'a mut [u8],
    /// The position where the allocated chunk header starts
    header_pos: usize,
    /// The size of the allocated header (the final header may be smaller)
    allocated_header: usize,
    /// The position of the data in the chunk
    pos: usize,
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
        }
    }

    /// Terminate the request body by writing an empty chunk
    pub async fn terminate(&mut self) -> Result<(), C::Error> {
        assert!(self.allocated_header > 0);

        if self.pos > self.header_pos + self.allocated_header {
            // There are bytes written in the current chunk
            self.finish_current_chunk();

            if self.header_pos + EMPTY_CHUNK.len() > self.buf.len() {
                // There is not enough space to fit the empty chunk in the buffer
                self.emit_finished_chunks().await?;
            }
        }

        self.buf[self.header_pos..self.header_pos + EMPTY_CHUNK.len()].copy_from_slice(EMPTY_CHUNK);
        self.header_pos += EMPTY_CHUNK.len();
        self.allocated_header = 0;
        self.pos = self.header_pos + self.allocated_header;
        self.emit_finished_chunks().await
    }

    /// Append to the buffer
    fn append_current_chunk(&mut self, buf: &[u8]) -> usize {
        let buffered = usize::min(buf.len(), self.buf.len() - NEWLINE.len() - self.pos);
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

    async fn emit_finished_chunks(&mut self) -> Result<(), C::Error> {
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
        let written = self.append_current_chunk(buf);
        if written < buf.len() {
            self.finish_current_chunk();
            self.emit_finished_chunks().await.map_err(|e| e.kind())?;
        }
        Ok(written)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        if self.pos > self.header_pos + self.allocated_header {
            // There are bytes written in the current chunk
            self.finish_current_chunk();
            self.emit_finished_chunks().await.map_err(|e| e.kind())?;
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

fn write_chunked_header(buf: &mut [u8], chunk_len: usize) -> usize {
    let mut hex = [0; 2 * size_of::<usize>()];
    hex::encode_to_slice(chunk_len.to_be_bytes(), &mut hex).unwrap();
    let leading_zeros = hex.iter().position(|x| *x != b'0').unwrap_or(hex.len() - 1);
    let hex_chars = hex.len() - leading_zeros;
    buf[..hex_chars].copy_from_slice(&hex[leading_zeros..]);
    buf[hex_chars..hex_chars + NEWLINE.len()].copy_from_slice(NEWLINE);
    hex_chars + 2
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
        assert_eq!(0, get_max_chunk_header_size(3));
        assert_eq!(3, get_max_chunk_header_size(0x00 + 2 + 2));
        assert_eq!(3, get_max_chunk_header_size(0x01 + 2 + 2));
        assert_eq!(3, get_max_chunk_header_size(0x0F + 2 + 2));
        assert_eq!(4, get_max_chunk_header_size(0x10 + 2 + 2));
        assert_eq!(4, get_max_chunk_header_size(0x11 + 2 + 2));
        assert_eq!(4, get_max_chunk_header_size(0x12 + 2 + 2));
    }

    #[test]
    fn can_write_chunked_header() {
        let mut buf = [0; 4];

        let len = write_chunked_header(&mut buf, 0x00);
        assert_eq!(b"0\r\n", &buf[..len]);

        let len = write_chunked_header(&mut buf, 0x01);
        assert_eq!(b"1\r\n", &buf[..len]);

        let len = write_chunked_header(&mut buf, 0x0F);
        assert_eq!(b"f\r\n", &buf[..len]);

        let len = write_chunked_header(&mut buf, 0x10);
        assert_eq!(b"10\r\n", &buf[..len]);
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
    async fn current_chunk_is_emitted_before_empty_chunk_is_emitted() {
        // Given
        let mut conn = Vec::new();
        let mut buf = [0; 14];
        buf[..5].copy_from_slice(b"HELLO");

        // When
        let mut writer = BufferingChunkedBodyWriter::new_with_data(&mut conn, &mut buf, 5);
        writer.write_all(b"BODY").await.unwrap(); // Can fit
        writer.terminate().await.unwrap(); // Cannot fit

        // Then
        assert_eq!(b"HELLO4\r\nBODY\r\n0\r\n\r\n", conn.as_slice());
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
