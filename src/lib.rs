extern crate futures;
extern crate bytes;
extern crate tokio_io;

use std::cmp;
use std::fs::File;
use std::io::{Read, Write, BufRead, BufReader, Lines};
use std::path::Path;
use std::time::{Instant, Duration};

use bytes::{Buf, BufMut};
use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};
//TODO use tokio_io::codec::{Decoder, Encoder, Framed};
//TODO use tokio_io::split::{ReadHalf, WriteHalf};


pub trait IoDump {
    fn write_block(&mut self, Direction, &[u8]) -> std::io::Result<()>;
}

/// Holds data needed by IoDump implementation.
pub struct Holder<T> {
    upstream: T,
    control: Enum,
}

pub enum Enum {
    Passthru,
    Logger ( Trace ),
}

pub struct Trace {
    dump: File,
    now:  Instant,
}
//--------------------------------------------------------------------------------------------------

impl IoDump {
    pub fn passthru<T>(upstream: T) -> std::io::Result<Holder<T>> {
        Ok(
            Holder {
                upstream: upstream,
                control: Enum::Passthru,
            })
    }

    pub fn wrapper<T, P: AsRef<Path>>(upstream: T, path: P) -> std::io::Result<Holder<T>> {
        Ok(
            Holder {
                upstream: upstream,
                control: Enum::Logger(
                    Trace {
                        dump: try!(File::create(path)),
                        now: Instant::now(),
                    }),
            })
    }
}

//--------------------------------------------------------------------------------------------------

impl Trace {
    fn write_block(&mut self, dir: Direction, data: &[u8]) -> std::io::Result<()> {
        if dir == Direction::In {
            try!(write!(self.dump, "<-  "));
        } else {
            try!(write!(self.dump, "->  "));
        }

        // Write elapsed time
        let elapsed = millis((Instant::now() - self.now)) as f64 / 1000.0;
        try!(write!(self.dump, "{:.*}s  {} bytes", 3, elapsed, data.len()));

        // Write newline
        try!(write!(self.dump, "\n"));

        let mut pos = 0;

        while pos < data.len() {
            let end = cmp::min(pos + LINE, data.len());

            //try!(self.write_data_line(&data[pos..end]));
            let line = &data[pos..end];

            // First write binary
            for i in 0..LINE {
                if i >= line.len() {
                    try!(write!(self.dump, "   "));
                } else {
                    try!(write!(self.dump, "{:02X} ", line[i]));
                }
            }

            // Write some spacing for the ascii
            try!(write!(self.dump, "    "));

            for &byte in line.iter() {
                match byte {
                     0 => try!(write!(self.dump, "\\0")),
                     9 => try!(write!(self.dump, "\\t")),
                    10 => try!(write!(self.dump, "\\n")),
                    13 => try!(write!(self.dump, "\\r")),
                    32...126 => {
                        try!(self.dump.write(&[b' ', byte]));
                    }
                    _ => try!(write!(self.dump, "\\?")),
                }
            }

            try!(write!(self.dump, "\n"));

            pos = end;
        }

        try!(write!(self.dump, "\n"));

        Ok(())
    }
}

//--------------------------------------------------------------------------------------------------

impl<T> IoDump for Holder<T> {
    fn write_block(&mut self, dir: Direction, data: &[u8]) -> std::io::Result<()> {
        match self.control {
            Enum::Logger( ref mut trace ) => try!(trace.write_block(dir, &data)),
            Enum::Passthru                => (),
        }
        Ok(())
    }
}

//--------------------------------------------------------------------------------------------------

impl<T: Read> Read for Holder<T> {
    fn read(&mut self, dst: &mut [u8]) -> std::io::Result<usize> {
        let n = try!(self.upstream.read(dst));
        match self.control {
            Enum::Logger( ref mut trace ) => try!(trace.write_block(Direction::Out, &dst[0..n])),
            Enum::Passthru                => (),
        }
        Ok(n)
    }
}

impl<T: Write> Write for Holder<T> {
    fn write(&mut self, src: &[u8]) -> std::io::Result<usize> {
        let n = try!(self.upstream.write(src));
        match self.control {
            Enum::Logger( ref mut trace ) => try!(trace.write_block(Direction::In, &src[0..n])),
            Enum::Passthru                => (),
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        try!(self.upstream.flush());
        Ok(())
    }
}

impl<T: AsyncRead> AsyncRead for Holder<T> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.upstream.prepare_uninitialized_buffer(buf)
    }

    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, std::io::Error>
        where Self: Sized,
    {
        //TODO: unable to wait for completion? unable to call write_block ?
        self.upstream.read_buf(buf)
    }

    //TODO fn framed<T: Encoder + Decoder>(self, codec: T) -> Framed<Self, T> {
    //TODO     self.holder.upstream.framed(codec)
    //TODO }

    //TODO fn split(self) -> (ReadHalf<Self>, WriteHalf<Self>) {
    //TODO     self.holder.upstream.split()
    //TODO }
}

impl<'t,T: AsyncWrite> AsyncWrite for Holder<T> {
    fn shutdown(&mut self) -> Poll<(), std::io::Error> {
        self.upstream.shutdown()
    }

    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, std::io::Error> {
        //TODO: unable to wait for completion? unable to call write_block ?
        self.upstream.write_buf(buf)
    }
}

//--------------------------------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Direction {
    In,
    Out,
}

//--------------------------------------------------------------------------------------------------

const LINE: usize = 25;
const NANOS_PER_MILLI: u32 = 1_000_000;
const MILLIS_PER_SEC: u64 = 1_000;

/// Convert a `Duration` to milliseconds, rounding up and saturating at
/// `u64::MAX`.
///
/// The saturating is fine because `u64::MAX` milliseconds are still many
/// million years.
pub fn millis(duration: Duration) -> u64 {
    // Round up.
    let millis = (duration.subsec_nanos() + NANOS_PER_MILLI - 1) / NANOS_PER_MILLI;
    duration.as_secs().saturating_mul(MILLIS_PER_SEC).saturating_add(millis as u64)
}





//--------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Block {
    head: Head,
    data: Vec<u8>,
}

#[derive(Debug)]
struct Head {
    direction: Direction,
    elapsed: Duration,
}

impl Block {
    pub fn direction(&self) -> Direction {
        self.head.direction
    }

    pub fn elapsed(&self) -> Duration {
        self.head.elapsed
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/*
 *
 * ===== impl Dump =====
 *
 */

pub struct Dump {
    lines: Lines<BufReader<File>>,
}

impl Dump {
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Dump> {
        let dump = try!(File::open(path));
        let dump = BufReader::new(dump);
        Ok(Dump { lines: dump.lines() })
    }

    fn read_block(&mut self) -> std::io::Result<Option<Block>> {
        loop {
            let head = match self.lines.next() {
                Some(Ok(line)) => line,
                Some(Err(e)) => return Err(e),
                None => return Ok(None),
            };

            let head: Vec<String> = head
                .split(|v| v == ' ')
                .filter(|v| !v.is_empty())
                .map(|v| v.into())
                .collect();

            if head.len() == 0 || head[0] == "//" {
                continue;
            }

            assert_eq!(4, head.len());

            let dir = match &head[0][..] {
                "<-" => Direction::In,
                "->" => Direction::Out,
                _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid direction format")),
            };

            let elapsed: f64 = {
                let s = &head[1];
                s[..s.len()-1].parse().unwrap()
            };

            // Do nothing w/ bytes for now

            // ready body
            let mut data = vec![];

            loop {
                let line = match self.lines.next() {
                    Some(Ok(line)) => line,
                    Some(Err(e)) => return Err(e),
                    None => "".into(),
                };

                if line.is_empty() {
                    return Ok(Some(Block {
                        head: Head {
                            direction: dir,
                            elapsed: Duration::from_millis((elapsed * 1000.0) as u64),
                        },
                        data: data,
                    }));
                }

                let mut pos = 0;

                loop {
                    let c = &line[pos..pos+2];

                    if c == "  " {
                        break;
                    }

                    let byte = match u8::from_str_radix(c, 16) {
                        Ok(byte) => byte,
                        Err(_) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "could not parse byte")),
                    };

                    data.push(byte);

                    pos += 3;
                }
            }
        }
    }
}

impl Iterator for Dump {
    type Item = Block;

    fn next(&mut self) -> Option<Block> {
        self.read_block().unwrap()
    }
}


