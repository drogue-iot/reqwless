use embedded_io::{ErrorKind, ErrorType};
use embedded_io_async::Read;

pub struct ConcatReader<A, B>
where
    A: Read,
    B: Read,
{
    first: A,
    last: B,
    first_exhausted: bool,
}

impl<A, B> ConcatReader<A, B>
where
    A: Read,
    B: Read,
{
    pub const fn new(first: A, last: B) -> Self {
        Self {
            first,
            last,
            first_exhausted: false,
        }
    }
}

pub enum ConcatReaderError<A, B>
where
    A: Read,
    B: Read,
{
    First(A::Error),
    Last(B::Error),
}

impl<A, B> core::fmt::Debug for ConcatReaderError<A, B>
where
    A: Read,
    B: Read,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::First(arg0) => f.debug_tuple("First").field(arg0).finish(),
            Self::Last(arg0) => f.debug_tuple("Last").field(arg0).finish(),
        }
    }
}

impl<A, B> embedded_io::Error for ConcatReaderError<A, B>
where
    A: Read,
    B: Read,
{
    fn kind(&self) -> ErrorKind {
        match self {
            ConcatReaderError::First(a) => a.kind(),
            ConcatReaderError::Last(b) => b.kind(),
        }
    }
}

impl<A, B> ErrorType for ConcatReader<A, B>
where
    A: Read,
    B: Read,
{
    type Error = ConcatReaderError<A, B>;
}

impl<A, B> Read for ConcatReader<A, B>
where
    A: Read,
    B: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if !self.first_exhausted {
            let len = self.first.read(buf).await.map_err(ConcatReaderError::First)?;
            if len > 0 {
                return Ok(len);
            }

            self.first_exhausted = true;
        }

        self.last.read(buf).await.map_err(ConcatReaderError::Last)
    }
}
