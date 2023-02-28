use embedded_io::asynch::{Read, Write};

pub struct BufferedWrite<'buf, C: Write> {
    inner: C,
    buf: &'buf mut [u8],
    pos: usize,
}

impl<'buf, C: Write> BufferedWrite<'buf, C> {
    pub fn new(inner: C, buf: &'buf mut [u8]) -> Self {
        Self { inner, buf, pos: 0 }
    }
}

impl<C: Write> embedded_io::Io for BufferedWrite<'_, C> {
    type Error = C::Error;
}

impl<C: Read + Write> Read for BufferedWrite<'_, C> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.inner.read(buf).await
    }
}

impl<C: Write> Write for BufferedWrite<'_, C> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pos == 0 && buf.len() >= self.buf.len() {
            // Fast path - nothing in buffer and the buffer to write is large
            return self.inner.write(buf).await;
        }

        let buffered = usize::min(buf.len(), self.buf.len() - self.pos);
        if buffered > 0 {
            self.buf[self.pos..self.pos + buffered].copy_from_slice(&buf[..buffered]);
            self.pos += buffered;
        }

        if self.pos == self.buf.len() {
            // The buffer is full
            let written = self.inner.write(self.buf).await?;
            if written < self.pos {
                self.buf.copy_within(written..self.pos, 0);
                self.pos -= written;
            } else {
                self.pos = 0;
            }
        }

        Ok(buffered)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        if self.pos > 0 {
            self.inner.write_all(&self.buf[..self.pos]).await?;
            self.pos = 0;
        }

        self.inner.flush().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn can_append_to_buffer() {
        let mut inner = Vec::new();
        let mut buf = [0; 8];
        let mut buffered = BufferedWrite::new(&mut inner, &mut buf);

        assert_eq!(2, buffered.write(&[1, 2]).await.unwrap());
        assert_eq!(2, buffered.pos);
        assert_eq!(0, buffered.inner.len());

        assert_eq!(2, buffered.write(&[3, 4]).await.unwrap());
        assert_eq!(4, buffered.pos);
        assert_eq!(0, buffered.inner.len());

        assert_eq!(4, buffered.write(&[5, 6, 7, 8]).await.unwrap());
        assert_eq!(0, buffered.pos);
        assert_eq!(8, buffered.inner.len());
        assert_eq!(&[1, 2, 3, 4, 5, 6, 7, 8], buffered.inner.as_slice());
    }

    #[tokio::test]
    async fn bypass_large_write_when_empty() {
        let mut inner = Vec::new();
        let mut buf = [0; 8];
        let mut buffered = BufferedWrite::new(&mut inner, &mut buf);

        assert_eq!(8, buffered.write(&[1, 2, 3, 4, 5, 6, 7, 8]).await.unwrap());
        assert_eq!(0, buffered.pos);
        assert_eq!(8, buffered.inner.len());
    }

    #[tokio::test]
    async fn large_write_when_not_empty() {
        let mut inner = Vec::new();
        let mut buf = [0; 8];
        let mut buffered = BufferedWrite::new(&mut inner, &mut buf);

        assert_eq!(1, buffered.write(&[1]).await.unwrap());
        assert_eq!(1, buffered.pos);
        assert_eq!(0, buffered.inner.len());

        assert_eq!(7, buffered.write(&[2, 3, 4, 5, 6, 7, 8, 9]).await.unwrap());
        assert_eq!(0, buffered.pos);
        assert_eq!(8, buffered.inner.len());
    }

    #[tokio::test]
    async fn flush_clears_buffer() {
        let mut inner = Vec::new();
        let mut buf = [0; 8];
        let mut buffered = BufferedWrite::new(&mut inner, &mut buf);

        assert_eq!(2, buffered.write(&[1, 2]).await.unwrap());
        assert_eq!(2, buffered.pos);
        assert_eq!(0, buffered.inner.len());

        buffered.flush().await.unwrap();
        assert_eq!(0, buffered.pos);
        assert_eq!(2, buffered.inner.len());
    }
}
