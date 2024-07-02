use core::mem::size_of;

use embedded_io::{Error, ErrorType};
use embedded_io_async::Write;

pub struct ChunkedBodyWriter<C: Write>(C);

const EMPTY_CHUNK: &[u8; 5] = b"0\r\n\r\n";
const NEWLINE: &[u8; 2] = b"\r\n";

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

pub(super) fn write_chunked_header(buf: &mut [u8], chunk_len: usize) -> usize {
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
}
