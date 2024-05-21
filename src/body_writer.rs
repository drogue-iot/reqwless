use core::mem::size_of;

use embedded_io::{Error as _, ErrorType};
use embedded_io_async::Write;

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

    pub async fn terminate(&mut self) -> Result<(), C::Error> {
        self.0.write_all(b"0\r\n\r\n").await
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
        self.0.write_all(b"\r\n").await.map_err(|e| e.kind())?;
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await.map_err(|e| e.kind())
    }
}

pub struct BufferingChunkedBodyWriter<'a, C: Write> {
    conn: C,
    buf: &'a mut [u8],
    header_pos: usize,
    pos: usize,
    max_header_size: usize,
    max_footer_size: usize,
}

impl<'a, C> BufferingChunkedBodyWriter<'a, C>
where
    C: Write,
{
    pub fn new_with_data(conn: C, buf: &'a mut [u8], written: usize) -> Self {
        let max_hex_chars = hex_chars(buf.len());
        let max_header_size = max_hex_chars as usize + 2;
        let max_footer_size = 2;
        assert!(buf.len() > max_header_size + max_footer_size); // There must be space for the chunk header and footer
        Self {
            conn,
            buf,
            header_pos: written,
            pos: written + max_header_size,
            max_header_size,
            max_footer_size,
        }
    }

    pub async fn terminate(&mut self) -> Result<(), C::Error> {
        if self.pos > self.header_pos + self.max_header_size {
            self.finish_current_chunk();
        }
        const EMPTY: &[u8; 5] = b"0\r\n\r\n";
        if self.header_pos + EMPTY.len() > self.buf.len() {
            self.emit_finished_chunk().await?;
        }

        self.buf[self.header_pos..self.header_pos + EMPTY.len()].copy_from_slice(EMPTY);
        self.header_pos += EMPTY.len();
        self.pos = self.header_pos + self.max_header_size;
        self.emit_finished_chunk().await
    }

    fn append_current_chunk(&mut self, buf: &[u8]) -> usize {
        let buffered = usize::min(buf.len(), self.buf.len() - self.max_footer_size - self.pos);
        if buffered > 0 {
            self.buf[self.pos..self.pos + buffered].copy_from_slice(&buf[..buffered]);
            self.pos += buffered;
        }
        buffered
    }

    fn finish_current_chunk(&mut self) {
        // Write the header in the allocated position position
        let chunk_len = self.pos - self.header_pos - self.max_header_size;
        let header_buf = &mut self.buf[self.header_pos..self.header_pos + self.max_header_size];
        let header_len = write_chunked_header(header_buf, chunk_len);

        // Move the payload if the header length was not as large as it could possibly be
        let spacing = self.max_header_size - header_len;
        if spacing > 0 {
            self.buf.copy_within(
                self.header_pos + self.max_header_size..self.pos,
                self.header_pos + header_len,
            );
            self.pos -= spacing
        }

        // Write newline footer after chunk payload
        self.buf[self.pos..self.pos + 2].copy_from_slice(b"\r\n");
        self.pos += 2;

        self.header_pos = self.pos;
        self.pos = self.header_pos + self.max_header_size;
    }

    async fn emit_finished_chunk(&mut self) -> Result<(), C::Error> {
        self.conn.write_all(&self.buf[..self.header_pos]).await?;
        self.header_pos = 0;
        self.pos = self.max_header_size;
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
            self.emit_finished_chunk().await.map_err(|e| e.kind())?;
        }
        Ok(written)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        if self.header_pos > 0 {
            self.finish_current_chunk();
            self.emit_finished_chunk().await.map_err(|e| e.kind())?;
        }
        self.conn.flush().await.map_err(|e| e.kind())
    }
}

const fn hex_chars(number: usize) -> u32 {
    if number == 0 {
        1
    } else {
        (usize::BITS - number.leading_zeros()).div_ceil(4)
    }
}

fn write_chunked_header(buf: &mut [u8], chunk_len: usize) -> usize {
    let mut hex = [0; 2 * size_of::<usize>()];
    hex::encode_to_slice(chunk_len.to_be_bytes(), &mut hex).unwrap();
    let leading_zeros = hex.iter().position(|x| *x != b'0').unwrap_or_default();
    let hex_chars = hex.len() - leading_zeros;
    buf[..hex_chars].copy_from_slice(&hex[leading_zeros..]);
    buf[hex_chars..hex_chars + 2].copy_from_slice(b"\r\n");
    hex_chars + 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_chars_values() {
        assert_eq!(1, hex_chars(0));
        assert_eq!(1, hex_chars(1));
        assert_eq!(1, hex_chars(0xF));
        assert_eq!(2, hex_chars(0x10));
        assert_eq!(2, hex_chars(0xFF));
        assert_eq!(3, hex_chars(0x100));
    }
}
