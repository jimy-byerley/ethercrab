use crate::generate::le_u16;
use core::fmt;
use nom::{combinator::map, error::ParseError, sequence::pair, IResult};

/// PDU command.
#[derive(Default, PartialEq, Eq, Debug, Copy, Clone)]
pub enum Command {
    /// No operation.
    #[default]
    Nop,

    /// APRD.
    Aprd {
        /// Auto increment counter.
        address: u16,

        /// Memory location to read from.
        register: u16,
    },
    /// FPRD.
    Fprd {
        /// Configured station address.
        address: u16,

        /// Memory location to read from.
        register: u16,
    },
    /// BRD.
    Brd {
        /// Autoincremented by each slave visited.
        address: u16,

        /// Memory location to read from.
        register: u16,
    },
    /// LRD.
    Lrd {
        /// Logical address.
        address: u32,
    },

    /// BWR.
    Bwr {
        /// Autoincremented by each slave visited.
        address: u16,

        /// Memory location to write to.
        register: u16,
    },
    /// APWR.
    Apwr {
        /// Auto increment counter.
        address: u16,

        /// Memory location to write to.
        register: u16,
    },
    /// FPWR.
    Fpwr {
        /// Configured station address.
        address: u16,

        /// Memory location to read from.
        register: u16,
    },
    /// FRMW.
    Frmw {
        /// Configured station address.
        address: u16,

        /// Memory location to read from.
        register: u16,
    },
    /// LWR.
    Lwr {
        /// Logical address.
        address: u32,
    },

    /// LRW.
    Lrw {
        /// Logical address.
        address: u32,
    },
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Nop => write!(f, "NOP"),
            Command::Aprd { address, register } => {
                write!(f, "APRD(addr {}, reg {})", address, register)
            }
            Command::Fprd { address, register } => {
                write!(f, "FPRD(addr {}, reg {}", address, register)
            }
            Command::Brd { address, register } => {
                write!(f, "BRD(addr {}, reg {}", address, register)
            }
            Command::Lrd { address } => write!(f, "LRD(addr {})", address),
            Command::Bwr { address, register } => {
                write!(f, "BWR(addr {}, reg {}", address, register)
            }
            Command::Apwr { address, register } => {
                write!(f, "APWR(addr {}, reg {}", address, register)
            }
            Command::Fpwr { address, register } => {
                write!(f, "FPWR(addr {}, reg {}", address, register)
            }
            Command::Frmw { address, register } => {
                write!(f, "FRMW(addr {}, reg {}", address, register)
            }
            Command::Lwr { address } => write!(f, "LWR(addr {})", address),
            Command::Lrw { address } => write!(f, "LRW(addr {})", address),
        }
    }
}

impl Command {
    /// Get just the command code for a command.
    pub const fn code(&self) -> CommandCode {
        match self {
            Self::Nop => CommandCode::Nop,

            // Reads
            Self::Aprd { .. } => CommandCode::Aprd,
            Self::Fprd { .. } => CommandCode::Fprd,
            Self::Brd { .. } => CommandCode::Brd,
            Self::Lrd { .. } => CommandCode::Lrd,

            // Writes
            Self::Bwr { .. } => CommandCode::Bwr,
            Self::Apwr { .. } => CommandCode::Apwr,
            Self::Fpwr { .. } => CommandCode::Fpwr,
            Self::Lwr { .. } => CommandCode::Lwr,
            Self::Frmw { .. } => CommandCode::Frmw,

            // Read/writes
            Self::Lrw { .. } => CommandCode::Lrw,
        }
    }

    /// Get the address value for the command.
    pub fn address(&self) -> [u8; 4] {
        let mut arr = [0x00u8; 4];

        let buf = arr.as_mut_slice();

        match *self {
            Command::Nop => arr,

            Command::Aprd { address, register }
            | Command::Apwr { address, register }
            | Command::Fprd { address, register }
            | Command::Fpwr { address, register }
            | Command::Frmw { address, register }
            | Command::Brd { address, register }
            | Command::Bwr { address, register } => {
                let buf = le_u16(address, buf);
                let _buf = le_u16(register, buf);

                arr
            }
            Command::Lrd { address } | Command::Lwr { address } | Command::Lrw { address } => {
                address.to_le_bytes()
            }
        }
    }
}

/// Broadcast or configured station addressing.
#[derive(Default, Copy, Clone, Debug, PartialEq, Eq, num_enum::TryFromPrimitive)]
#[repr(u8)]
pub enum CommandCode {
    #[default]
    Nop = 0x00,

    // Reads
    Aprd = 0x01,
    Fprd = 0x04,
    Brd = 0x07,
    Lrd = 0x0A,

    // Writes
    Bwr = 0x08,
    Apwr = 0x02,
    Fpwr = 0x05,
    Frmw = 0x0E,
    Lwr = 0x0B,

    // Read/writes
    Lrw = 0x0c,
}

impl CommandCode {
    /// Parse an address, producing a [`Command`].
    pub fn parse_address<'a, E>(self, i: &'a [u8]) -> IResult<&'a [u8], Command, E>
    where
        E: ParseError<&'a [u8]>,
    {
        use nom::number::complete::{le_u16, le_u32};

        match self {
            Self::Nop => Ok((i, Command::Nop)),

            Self::Aprd => map(pair(le_u16, le_u16), |(address, register)| Command::Aprd {
                address,
                register,
            })(i),
            Self::Fprd => map(pair(le_u16, le_u16), |(address, register)| Command::Fprd {
                address,
                register,
            })(i),
            Self::Brd => map(pair(le_u16, le_u16), |(address, register)| Command::Brd {
                address,
                register,
            })(i),
            Self::Lrd => map(le_u32, |address| Command::Lrd { address })(i),

            Self::Bwr => map(pair(le_u16, le_u16), |(address, register)| Command::Bwr {
                address,
                register,
            })(i),
            Self::Apwr => map(pair(le_u16, le_u16), |(address, register)| Command::Apwr {
                address,
                register,
            })(i),
            Self::Fpwr => map(pair(le_u16, le_u16), |(address, register)| Command::Fpwr {
                address,
                register,
            })(i),
            Self::Frmw => map(pair(le_u16, le_u16), |(address, register)| Command::Frmw {
                address,
                register,
            })(i),
            Self::Lwr => map(le_u32, |address| Command::Lwr { address })(i),

            Self::Lrw => map(le_u32, |address| Command::Lrw { address })(i),
        }
    }
}
