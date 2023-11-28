use embedded_io_async::{BufRead, Error as _, ErrorType, Read};

use crate::Error;

/// Fixed length response body reader
pub struct FixedLengthBodyReader<B> {
    pub raw_body: B,
    pub remaining: usize,
}

impl<C> ErrorType for FixedLengthBodyReader<C> {
    type Error = Error;
}

impl<C> Read for FixedLengthBodyReader<C>
where
    C: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        if self.remaining == 0 {
            return Ok(0);
        }

        let read = self.raw_body.read(buf).await.map_err(|e| Error::Network(e.kind()))?;
        self.remaining -= read;

        Ok(read)
    }
}

impl<C> BufRead for FixedLengthBodyReader<C>
where
    C: BufRead + Read,
{
    async fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
        if self.remaining == 0 {
            return Ok(&[]);
        }

        let loaded = self
            .raw_body
            .fill_buf()
            .await
            .map_err(|e| Error::Network(e.kind()))
            .map(|data| &data[..data.len().min(self.remaining)])?;

        if loaded.is_empty() {
            return Err(Error::ConnectionAborted);
        }

        Ok(loaded)
    }

    fn consume(&mut self, amt: usize) {
        let amt = amt.min(self.remaining);
        self.remaining -= amt;
        self.raw_body.consume(amt)
    }
}
