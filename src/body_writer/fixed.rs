use embedded_io::ErrorType;
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
