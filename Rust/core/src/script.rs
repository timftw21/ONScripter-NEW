use flate2::read::ZlibDecoder;
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    mem,
    ops::Range,
};

use crate::{Error, Limits, Result, read_limited};

// Inverse of nscmake's conversion table; this is an encoding, not authentication.
const CONVERSION: [u8; 256] = [
    0x37, 0x6a, 0x09, 0x5e, 0x7a, 0xaf, 0xf5, 0xa4, 0xba, 0x78, 0x84, 0x58, 0x35, 0x1e, 0x6b, 0x0c,
    0x49, 0xc6, 0xc3, 0x44, 0x40, 0x9e, 0x6f, 0x65, 0xe4, 0xf6, 0xfe, 0x22, 0xe2, 0x95, 0xc7, 0x38,
    0xf0, 0x1a, 0x82, 0xe0, 0x5b, 0x2a, 0xd8, 0xe5, 0xce, 0x2f, 0x74, 0x25, 0xec, 0x59, 0xc0, 0x45,
    0x4b, 0x64, 0x43, 0xdc, 0xb0, 0xb9, 0x30, 0x6d, 0x28, 0xd1, 0x16, 0xbb, 0x66, 0x98, 0x92, 0x90,
    0x2c, 0xa7, 0xf1, 0x80, 0xc1, 0xd4, 0x8b, 0xd6, 0xdf, 0x24, 0x2d, 0xf7, 0xfb, 0x88, 0x4d, 0x3c,
    0x72, 0xf3, 0xdb, 0x2b, 0x93, 0x73, 0xef, 0x85, 0x83, 0xee, 0xc2, 0x8d, 0x5c, 0xb2, 0x0b, 0x94,
    0x3d, 0xa8, 0x3f, 0x1c, 0x4c, 0x6e, 0x03, 0x7b, 0x1d, 0x5a, 0x51, 0xa1, 0x70, 0x41, 0xd0, 0xaa,
    0xa0, 0x7e, 0xcd, 0xd5, 0x15, 0xa9, 0x18, 0x76, 0xc9, 0x7d, 0x7f, 0x0e, 0x3a, 0x99, 0xbf, 0xab,
    0x3b, 0x14, 0x3e, 0x9a, 0x04, 0xda, 0x02, 0xfd, 0x63, 0xd9, 0xfa, 0x9f, 0x4e, 0xe3, 0x61, 0xbe,
    0x07, 0x11, 0xa6, 0x1b, 0x19, 0x55, 0x8e, 0x77, 0x0a, 0x47, 0xe6, 0xf8, 0x0d, 0xcf, 0xd7, 0x33,
    0x23, 0x1f, 0xbc, 0x62, 0xde, 0x9b, 0x29, 0x53, 0x68, 0xe8, 0x21, 0xb6, 0x34, 0x52, 0x87, 0xcb,
    0x08, 0x79, 0xf4, 0x67, 0x69, 0x54, 0xe7, 0x86, 0xea, 0xb4, 0x20, 0x71, 0x01, 0xbd, 0x06, 0x31,
    0x00, 0x50, 0xc8, 0xb8, 0xac, 0x5d, 0x57, 0x7c, 0x89, 0xeb, 0xb7, 0x36, 0x8f, 0xf2, 0xe1, 0x56,
    0x81, 0x4a, 0xd2, 0x8c, 0xf9, 0xad, 0x60, 0xa5, 0x42, 0x10, 0x5f, 0x12, 0xb3, 0xff, 0x4f, 0xdd,
    0x46, 0x26, 0xa2, 0x17, 0xc5, 0x75, 0x91, 0x27, 0xb5, 0x8a, 0xd3, 0x13, 0x2e, 0xc4, 0xe9, 0x9d,
    0x97, 0x39, 0x32, 0x05, 0x0f, 0xca, 0xcc, 0x48, 0xfc, 0xae, 0x96, 0xed, 0x6c, 0x9c, 0xb1, 0xa3,
];

pub struct ScriptSource {
    pub text: String,
    pub compressed: bool,
    pub encoded_bytes: usize,
}

impl ScriptSource {
    pub fn read(reader: &mut impl Read, limits: Limits) -> Result<Self> {
        let maximum = limits
            .script_bytes
            .checked_add(16)
            .ok_or(Error::Limit("script size"))?;
        let mut bytes = read_limited(reader, maximum)?;
        let encoded_bytes = bytes.len();
        let compressed = bytes.starts_with(b"ONS2");
        if compressed {
            if bytes.len() < 16 {
                return Err(Error::invalid("truncated compressed-script header"));
            }
            let header = |offset| {
                u32::from_le_bytes([
                    bytes[offset],
                    bytes[offset + 1],
                    bytes[offset + 2],
                    bytes[offset + 3],
                ]) as usize
            };
            let stored = header(4);
            let original = header(8);
            if header(12) != 110 {
                return Err(Error::invalid("unsupported compressed-script version"));
            }
            if stored < 16
                || original < 16
                || stored > limits.script_bytes
                || original > limits.script_bytes
            {
                return Err(Error::Limit("compressed script size"));
            }
            if stored != bytes.len() - 16 {
                return Err(Error::invalid(
                    "compressed-script payload size differs from its header",
                ));
            }
            for byte in &mut bytes[16..] {
                *byte = CONVERSION[usize::from(*byte ^ 0x86)] ^ 0x23;
            }
            let mut decoder = ZlibDecoder::new(&bytes[16..]);
            let mut decoded = read_limited(&mut decoder, original)?;
            if decoded.len() != original || decoder.total_in() as usize != stored {
                return Err(Error::invalid(
                    "compressed-script decoded size differs from its header",
                ));
            }
            for byte in &mut decoded {
                *byte = CONVERSION[usize::from(*byte ^ 0x45)] ^ 0x71;
            }
            bytes = decoded;
        } else if bytes.len() > limits.script_bytes {
            return Err(Error::Limit("script size"));
        }
        if bytes.last() == Some(&0) {
            bytes.pop();
        }
        if bytes.contains(&0) {
            return Err(Error::invalid("script contains an embedded NUL"));
        }
        let mut text =
            String::from_utf8(bytes).map_err(|_| Error::invalid("script is not valid UTF-8"))?;
        if text.starts_with('\u{feff}') {
            text.drain(..3);
        }
        if text.trim().is_empty() {
            return Err(Error::invalid("script is empty"));
        }
        Ok(Self {
            text,
            compressed,
            encoded_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opcode {
    Label,
    Marker,
    JumpForward,
    JumpBackward,
    Game,
    End,
    Caption,
    Mov,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Inc,
    Dec,
    Goto,
    Gosub,
    GetParam,
    Return,
    If,
    NotIf,
    NumAlias,
    StrAlias,
    DefSub,
    Wait,
    Click,
    Archives,
    ArchiveDirectory,
    FileExists,
    Scene,
    Effect,
    Dialogue,
    Text,
    Other,
}

impl Opcode {
    pub fn parse(name: &str) -> Self {
        match name
            .strip_prefix('_')
            .unwrap_or(name)
            .to_ascii_lowercase()
            .as_str()
        {
            "game" => Self::Game,
            "end" => Self::End,
            "caption" => Self::Caption,
            "mov" => Self::Mov,
            "add" => Self::Add,
            "sub" => Self::Sub,
            "mul" => Self::Mul,
            "div" => Self::Div,
            "mod" => Self::Mod,
            "inc" => Self::Inc,
            "dec" => Self::Dec,
            "goto" => Self::Goto,
            "jumpf" => Self::JumpForward,
            "jumpb" => Self::JumpBackward,
            "gosub" => Self::Gosub,
            "getparam" => Self::GetParam,
            "return" => Self::Return,
            "if" => Self::If,
            "notif" => Self::NotIf,
            "numalias" => Self::NumAlias,
            "stralias" => Self::StrAlias,
            "defsub" => Self::DefSub,
            "wait" | "delay" => Self::Wait,
            "click" => Self::Click,
            "nsa" | "ns2" | "ns3" => Self::Archives,
            "nsadir" => Self::ArchiveDirectory,
            "fileexist" => Self::FileExists,
            "bg" | "lsp" | "lsph" | "csp" | "vsp" | "msp" | "amsp" | "cell" | "allsphide"
            | "allspresume" | "print" => Self::Scene,
            "effect" => Self::Effect,
            "d" | "d2" => Self::Dialogue,
            "setwindow" | "setwindow3" | "setwindow4" | "setwindow2" | "text_speed" | "texton"
            | "textoff" | "textshow" | "texthide" | "textclear" | "br" | "preset_define"
            | "d_condition" | "d_continue" | "d_dispose" | "wait_on_d" => Self::Text,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Location {
    pub line: u32,
    pub column: u32,
    pub byte: usize,
}

#[derive(Debug)]
pub struct Instruction {
    pub opcode: Opcode,
    pub location: Location,
    name: Range<usize>,
    arguments: Range<usize>,
}

pub struct Program {
    source: ScriptSource,
    instructions: Vec<Instruction>,
    labels: HashMap<String, usize>,
    markers: Vec<usize>,
}

impl Program {
    pub fn parse(source: ScriptSource, limits: Limits) -> Result<Self> {
        if source.text.len() > limits.script_bytes {
            return Err(Error::Limit("script size"));
        }
        let mut program = Self {
            source,
            instructions: Vec::new(),
            labels: HashMap::new(),
            markers: Vec::new(),
        };
        let mut offset = 0;
        let mut metadata_bytes = 0usize;
        for (index, line) in program.source.text.split_inclusive('\n').enumerate() {
            let line_number =
                u32::try_from(index + 1).map_err(|_| Error::Limit("script line count"))?;
            let bytes = line.as_bytes();
            let mut cursor = 0;
            while cursor < bytes.len() {
                while cursor < bytes.len()
                    && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b':')
                {
                    cursor += 1;
                }
                if cursor == bytes.len() || bytes[cursor] == b';' {
                    break;
                }
                let start = cursor;
                let label = bytes[cursor] == b'*';
                let marker = bytes[cursor] == b'~';
                if label || marker {
                    cursor += 1;
                }
                if !marker
                    && (cursor == bytes.len()
                        || !(bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_'))
                {
                    return Err(Error::Script {
                        line: line_number,
                        message: "expected a command or label".into(),
                    });
                }
                while !marker
                    && cursor < bytes.len()
                    && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
                {
                    cursor += 1;
                }
                let name = (offset + start + usize::from(label))..(offset + cursor);
                let arguments_start = cursor;
                let mut quote = None;
                if !label
                    && !marker
                    && Opcode::parse(&program.source.text[name.clone()]) == Opcode::Dialogue
                {
                    cursor = line.trim_end_matches(['\r', '\n']).len();
                } else if !label && !marker {
                    while cursor < bytes.len() {
                        let byte = bytes[cursor];
                        match quote {
                            Some(delimiter) if delimiter == byte => quote = None,
                            Some(_) => {}
                            None if byte == b'"' || byte == b'`' => quote = Some(byte),
                            None if byte == b':' || byte == b';' || byte == b'\n' => break,
                            None => {}
                        }
                        cursor += 1;
                    }
                }
                let opcode = if marker {
                    Opcode::Marker
                } else if label {
                    Opcode::Label
                } else {
                    Opcode::parse(&program.source.text[name.clone()])
                };
                let cost = mem::size_of::<Instruction>() * 2
                    + if label { name.len() * 2 + 128 } else { 0 };
                metadata_bytes = metadata_bytes
                    .checked_add(cost)
                    .filter(|&n| n <= limits.program_bytes)
                    .ok_or(Error::Limit("compiled program"))?;
                program
                    .instructions
                    .try_reserve(1)
                    .map_err(|_| Error::Limit("program allocation"))?;
                if label {
                    program
                        .labels
                        .try_reserve(1)
                        .map_err(|_| Error::Limit("label allocation"))?;
                    program
                        .labels
                        .entry(program.source.text[name.clone()].to_ascii_lowercase())
                        .or_insert(program.instructions.len());
                }
                if marker {
                    program
                        .markers
                        .try_reserve(1)
                        .map_err(|_| Error::Limit("jump marker allocation"))?;
                    program.markers.push(program.instructions.len());
                }
                program.instructions.push(Instruction {
                    opcode,
                    name,
                    arguments: (offset + arguments_start)..(offset + cursor),
                    location: Location {
                        line: line_number,
                        column: (start + 1) as u32,
                        byte: offset + start,
                    },
                });
            }
            offset += line.len();
        }
        Ok(program)
    }

    pub fn label(&self, name: &str) -> Option<usize> {
        self.labels
            .get(&name.trim_start_matches('*').to_ascii_lowercase())
            .copied()
    }

    pub fn label_count(&self) -> usize {
        self.labels.len()
    }
    pub fn source(&self) -> &ScriptSource {
        &self.source
    }
    /// Match the maintained engine's declared script coordinate system.
    pub fn logical_size(&self) -> Result<(u32, u32)> {
        for line in self.source.text.lines() {
            let Some(header) = line.strip_prefix(';').or_else(|| line.strip_prefix(',')) else {
                break;
            };
            if let Some(mode) = header.strip_prefix("mode") {
                let end = mode
                    .find(|character: char| !character.is_ascii_digit())
                    .unwrap_or(mode.len());
                return match &mode[..end] {
                    "640" => Ok((640, 480)),
                    "800" => Ok((800, 600)),
                    "400" => Ok((400, 300)),
                    "320" => Ok((320, 240)),
                    "1920" => Ok((1920, 1080)),
                    "1280" => Ok((1280, 720)),
                    "480" => Ok((480, 272)),
                    _ => Err(Error::invalid("unsupported script canvas mode")),
                };
            }
        }
        Ok((1920, 1080))
    }
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }
    pub fn next_marker(&self, after: usize) -> Option<usize> {
        self.markers
            .get(self.markers.partition_point(|&marker| marker < after))
            .copied()
    }
    pub fn name<'a>(&'a self, instruction: &Instruction) -> &'a str {
        &self.source.text[instruction.name.clone()]
    }
    pub fn arguments<'a>(&'a self, instruction: &Instruction) -> &'a str {
        self.source.text[instruction.arguments.clone()].trim()
    }

    pub fn command_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for instruction in &self.instructions {
            if !matches!(instruction.opcode, Opcode::Label | Opcode::Marker) {
                *counts
                    .entry(self.name(instruction).to_ascii_lowercase())
                    .or_insert(0) += 1;
            }
        }
        counts
    }
}
