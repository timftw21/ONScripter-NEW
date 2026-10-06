use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use crate::{
    Error, Limits, Result,
    scene::{Effect, SceneCommand},
    script::{Opcode, Program},
    text::{MAX_PAGE_BYTES, TextCommand, TextPreset, TextWindow},
};

#[derive(Debug)]
pub enum Event {
    Yield,
    Blocked,
    Finished,
    Caption(String),
    Wait(u64),
    Click,
    Archives { directory: String, offset: u64 },
    FileExists { variable: u32, name: String },
    Scene { command: SceneCommand, line: u32 },
    Text { command: TextCommand, line: u32 },
}

#[derive(Default)]
struct Environment {
    numbers: HashMap<u32, i32>,
    strings: HashMap<u32, String>,
    number_aliases: HashMap<String, i32>,
    string_aliases: HashMap<String, String>,
    bytes: usize,
}

impl Environment {
    fn budget(&mut self, additional: usize, maximum: usize) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(additional)
            .filter(|&bytes| bytes <= maximum)
            .ok_or(Error::Limit("script state"))?;
        Ok(())
    }

    fn set_number(&mut self, variable: u32, value: i32, maximum: usize) -> Result<()> {
        if !self.numbers.contains_key(&variable) {
            self.budget(128, maximum)?;
            self.numbers
                .try_reserve(1)
                .map_err(|_| Error::Limit("variable allocation"))?;
        }
        self.numbers.insert(variable, value);
        Ok(())
    }

    fn set_string(&mut self, variable: u32, value: String, maximum: usize) -> Result<()> {
        let old = self.strings.get(&variable);
        let released = old.map_or(0, |value| value.len());
        let overhead = if old.is_none() { 128 } else { 0 };
        self.bytes -= released;
        self.budget(value.len() + overhead, maximum)?;
        self.strings
            .try_reserve(1)
            .map_err(|_| Error::Limit("string allocation"))?;
        self.strings.insert(variable, value);
        Ok(())
    }
}

struct Call<'a> {
    next: usize,
    arguments: &'a str,
}

struct Command<'a> {
    opcode: Opcode,
    name: &'a str,
    arguments: &'a str,
    line: u32,
    depth: usize,
}

pub struct Vm<'a> {
    program: &'a Program,
    limits: Limits,
    environment: Environment,
    subroutines: HashSet<String>,
    calls: Vec<Call<'a>>,
    pending: Option<Command<'a>>,
    pc: usize,
    defining: bool,
    blocked: bool,
    finished: bool,
    last_marker: Option<usize>,
    faulted: bool,
    archive_offset: u64,
    effects: HashMap<i32, Effect>,
}

impl<'a> Vm<'a> {
    pub fn new(program: &'a Program, limits: Limits) -> Self {
        Self {
            program,
            limits,
            environment: Environment::default(),
            subroutines: HashSet::new(),
            calls: Vec::new(),
            pending: None,
            pc: program.label("define").unwrap_or(0),
            defining: true,
            blocked: false,
            finished: false,
            last_marker: None,
            faulted: false,
            archive_offset: 0,
            effects: HashMap::new(),
        }
    }

    pub fn resume(&mut self) {
        self.blocked = false;
    }
    pub fn set_archive_offset(&mut self, offset: u64) {
        self.archive_offset = offset;
    }
    pub fn number(&self, variable: u32) -> i32 {
        self.environment
            .numbers
            .get(&variable)
            .copied()
            .unwrap_or(0)
    }
    pub fn set_number(&mut self, variable: u32, value: i32) -> Result<()> {
        self.environment
            .set_number(variable, value, self.limits.state_bytes)
    }

    /// Run bounded script work. Clocks, input and file access belong to the host.
    pub fn run_tick(&mut self) -> Result<Event> {
        let mut budget = self.limits.instructions_per_tick;
        self.run_with_budget(&mut budget)
    }

    /// Host callbacks share the same instruction budget for the whole frame.
    pub fn run_with_budget(&mut self, budget: &mut usize) -> Result<Event> {
        if self.faulted {
            return Err(Error::invalid("script stopped after an execution error"));
        }
        if self.finished {
            return Ok(Event::Finished);
        }
        if self.blocked {
            return Ok(Event::Blocked);
        }
        while *budget != 0 {
            *budget -= 1;
            let command = if let Some(command) = self.pending.take() {
                command
            } else {
                let Some(instruction) = self.program.instructions().get(self.pc) else {
                    return Err(Error::invalid("script reached EOF without an end command"));
                };
                self.pc += 1;
                Command {
                    opcode: instruction.opcode,
                    name: self.program.name(instruction),
                    arguments: self.program.arguments(instruction),
                    line: instruction.location.line,
                    depth: 0,
                }
            };
            let line = command.line;
            let event = match self.execute(command) {
                Ok(event) => event,
                Err(error) => {
                    self.faulted = true;
                    return Err(Error::Script {
                        line,
                        message: error.to_string(),
                    });
                }
            };
            if let Some(event) = event {
                return Ok(event);
            }
        }
        Ok(Event::Yield)
    }

    fn jump(&mut self, label: &str) -> Result<()> {
        self.pc = self
            .program
            .label(label)
            .ok_or_else(|| Error::invalid(format!("unknown label {label}")))?;
        Ok(())
    }

    fn call(&mut self, label: &str, arguments: &'a str) -> Result<()> {
        if self.calls.len() >= self.limits.call_depth {
            return Err(Error::Limit("call stack"));
        }
        let destination = self
            .program
            .label(label)
            .ok_or_else(|| Error::invalid(format!("unknown label {label}")))?;
        self.calls
            .try_reserve(1)
            .map_err(|_| Error::Limit("call stack allocation"))?;
        self.calls.push(Call {
            next: self.pc,
            arguments,
        });
        self.pc = destination;
        Ok(())
    }

    fn execute(&mut self, command: Command<'a>) -> Result<Option<Event>> {
        let Command {
            opcode,
            name,
            arguments: input,
            line,
            depth,
        } = command;
        if depth >= self.limits.expression_depth {
            return Err(Error::Limit("nested commands"));
        }
        let mut args = Arguments::new(input, self.limits);
        let builtin = name.starts_with('_');
        let name = name.strip_prefix('_').unwrap_or(name);
        let name = if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
            Cow::Owned(name.to_ascii_lowercase())
        } else {
            Cow::Borrowed(name)
        };
        // Maintained scripts can replace engine commands; _command forces the builtin.
        if !builtin
            && !matches!(opcode, Opcode::Label | Opcode::Marker)
            && self.subroutines.contains(name.as_ref())
        {
            self.call(&name, args.remaining())?;
            return Ok(None);
        }
        match opcode {
            Opcode::Label => {}
            Opcode::Marker => {
                self.last_marker = Some(self.pc);
            }
            Opcode::JumpForward => {
                args.finish()?;
                self.pc = self
                    .program
                    .next_marker(self.pc)
                    .map(|marker| marker + 1)
                    .ok_or_else(|| Error::invalid("jumpf has no following marker"))?;
            }
            Opcode::JumpBackward => {
                args.finish()?;
                self.pc = self
                    .last_marker
                    .ok_or_else(|| Error::invalid("jumpb has no executed marker"))?;
            }
            Opcode::Game => {
                args.finish()?;
                self.jump("start")?;
                self.defining = false;
            }
            Opcode::End => {
                args.finish()?;
                self.finished = true;
                return Ok(Some(Event::Finished));
            }
            Opcode::Caption => {
                let value = args.string(&self.environment)?;
                args.finish()?;
                return Ok(Some(Event::Caption(value)));
            }
            Opcode::Mov => {
                let (string, mut variable) = args.variable(&self.environment)?;
                args.comma()?;
                loop {
                    if string {
                        let value = args.string(&self.environment)?;
                        self.environment
                            .set_string(variable, value, self.limits.state_bytes)?;
                    } else {
                        let value = args.number(&self.environment)?;
                        self.set_number(variable, value)?;
                    }
                    if !args.eat(",") {
                        break;
                    }
                    variable = variable
                        .checked_add(1)
                        .filter(|&n| n <= i32::MAX as u32)
                        .ok_or_else(|| Error::invalid("variable index overflow"))?;
                }
                args.finish()?;
            }
            Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::Mod
            | Opcode::Inc
            | Opcode::Dec => {
                let (string, variable) = args.variable(&self.environment)?;
                if string {
                    return Err(Error::invalid(
                        "numeric operation requires an integer variable",
                    ));
                }
                let value = match opcode {
                    Opcode::Inc | Opcode::Dec => 1,
                    _ => {
                        args.comma()?;
                        args.number(&self.environment)?
                    }
                };
                args.finish()?;
                let result = arithmetic(
                    self.number(variable),
                    value,
                    match opcode {
                        Opcode::Add | Opcode::Inc => b'+',
                        Opcode::Sub | Opcode::Dec => b'-',
                        Opcode::Mul => b'*',
                        Opcode::Div => b'/',
                        _ => b'%',
                    },
                )?;
                self.set_number(variable, result)?;
            }
            Opcode::Goto | Opcode::Gosub => {
                let label = args.string(&self.environment)?;
                if opcode == Opcode::Goto {
                    args.finish()?;
                    self.jump(&label)?;
                } else {
                    if !args.remaining().is_empty() {
                        args.comma()?;
                    }
                    self.call(&label, args.remaining())?;
                }
            }
            Opcode::GetParam => {
                let caller = self
                    .calls
                    .last()
                    .ok_or_else(|| Error::invalid("getparam outside a subroutine"))?
                    .arguments;
                let mut provided = Arguments::new(caller, self.limits);
                loop {
                    let reference = args.eat("i") || args.eat("s");
                    let (string, variable) = args.variable(&self.environment)?;
                    if reference {
                        if string {
                            return Err(Error::invalid(
                                "variable reference requires an integer destination",
                            ));
                        }
                        let (_, index) = provided.variable(&self.environment)?;
                        self.set_number(variable, index as i32)?;
                    } else if string {
                        let value = provided.string(&self.environment)?;
                        self.environment
                            .set_string(variable, value, self.limits.state_bytes)?;
                    } else {
                        let value = provided.number(&self.environment)?;
                        self.set_number(variable, value)?;
                    }
                    if !provided.remaining().is_empty() {
                        provided.comma()?;
                    }
                    if !args.eat(",") {
                        break;
                    }
                }
                args.finish()?;
                self.calls
                    .last_mut()
                    .ok_or_else(|| Error::invalid("missing caller"))?
                    .arguments = provided.remaining();
            }
            Opcode::Return => {
                let destination = if args.remaining().is_empty() {
                    None
                } else {
                    Some(args.string(&self.environment)?)
                };
                args.finish()?;
                self.pc = self
                    .calls
                    .pop()
                    .ok_or_else(|| Error::invalid("return outside a subroutine"))?
                    .next;
                if let Some(label) = destination {
                    self.jump(&label)?;
                }
            }
            Opcode::If | Opcode::NotIf => {
                if args.conditions(&self.environment, opcode == Opcode::If)? {
                    if args.remaining().is_empty() {
                        return Ok(None);
                    }
                    let command = args.name()?;
                    self.pending = Some(Command {
                        opcode: Opcode::parse(command),
                        name: command,
                        arguments: args.remaining(),
                        line,
                        depth: depth + 1,
                    });
                    return Ok(None);
                }
                // RU skips the rest of the line, including colon commands.
                while self
                    .program
                    .instructions()
                    .get(self.pc)
                    .is_some_and(|instruction| instruction.location.line == line)
                {
                    self.pc += 1;
                }
            }
            Opcode::NumAlias | Opcode::StrAlias => {
                if !self.defining {
                    return Err(Error::invalid("aliases must be defined before game"));
                }
                let alias = args.name()?.to_ascii_lowercase();
                args.comma()?;
                if opcode == Opcode::NumAlias {
                    let value = args.number(&self.environment)?;
                    args.finish()?;
                    if !self.environment.number_aliases.contains_key(&alias) {
                        self.environment
                            .budget(alias.len() + 128, self.limits.state_bytes)?;
                    }
                    self.environment
                        .number_aliases
                        .try_reserve(1)
                        .map_err(|_| Error::Limit("alias allocation"))?;
                    self.environment.number_aliases.insert(alias, value);
                } else {
                    let value = args.string(&self.environment)?;
                    args.finish()?;
                    let old = self.environment.string_aliases.get(&alias);
                    let overhead = if old.is_none() { alias.len() + 128 } else { 0 };
                    self.environment.bytes -= old.map_or(0, |value| value.len());
                    self.environment
                        .budget(value.len() + overhead, self.limits.state_bytes)?;
                    self.environment
                        .string_aliases
                        .try_reserve(1)
                        .map_err(|_| Error::Limit("alias allocation"))?;
                    self.environment.string_aliases.insert(alias, value);
                }
            }
            Opcode::DefSub => {
                let subroutine = args.name()?.to_ascii_lowercase();
                args.finish()?;
                if !self.subroutines.contains(&subroutine) {
                    self.environment
                        .budget(subroutine.len() + 128, self.limits.state_bytes)?;
                }
                self.subroutines
                    .try_reserve(1)
                    .map_err(|_| Error::Limit("subroutine allocation"))?;
                self.subroutines.insert(subroutine);
            }
            Opcode::Wait => {
                let duration = args.number(&self.environment)?;
                args.finish()?;
                let duration = u64::try_from(duration)
                    .map_err(|_| Error::invalid("negative wait duration"))?;
                self.blocked = true;
                return Ok(Some(Event::Wait(duration)));
            }
            Opcode::Click => {
                args.finish()?;
                self.blocked = true;
                return Ok(Some(Event::Click));
            }
            Opcode::Archives | Opcode::ArchiveDirectory => {
                let directory = if opcode == Opcode::ArchiveDirectory {
                    args.string(&self.environment)?
                } else {
                    String::new()
                };
                args.finish()?;
                self.blocked = true;
                match name.as_ref() {
                    "ns2" => self.archive_offset = 1,
                    "ns3" => self.archive_offset = 2,
                    _ => {}
                }
                return Ok(Some(Event::Archives {
                    directory,
                    offset: self.archive_offset,
                }));
            }
            Opcode::FileExists => {
                let (string, variable) = args.variable(&self.environment)?;
                if string {
                    return Err(Error::invalid("fileexist requires an integer variable"));
                }
                args.comma()?;
                let name = args.string(&self.environment)?;
                args.finish()?;
                self.blocked = true;
                return Ok(Some(Event::FileExists { variable, name }));
            }
            Opcode::Dialogue => {
                if input.len() > MAX_PAGE_BYTES {
                    return Err(Error::Limit("dialogue"));
                }
                self.blocked = true;
                return Ok(Some(Event::Text {
                    command: if name == "d2" {
                        TextCommand::DialogueAsync(input.to_owned())
                    } else {
                        TextCommand::Dialogue(input.to_owned())
                    },
                    line,
                }));
            }
            Opcode::Text => {
                let command = match name.as_ref() {
                    "preset_define" => {
                        let (number, preset) = args.text_preset(&self.environment)?;
                        TextCommand::Preset { number, preset }
                    }
                    "d_condition" => {
                        let index = args.nonnegative(&self.environment)? as usize;
                        args.comma()?;
                        TextCommand::Condition {
                            index,
                            value: args.number(&self.environment)? == 1,
                        }
                    }
                    "d_continue" => TextCommand::ContinueDialogue,
                    "d_dispose" => TextCommand::DisposeDialogue,
                    "wait_on_d" => TextCommand::WaitDialogue(args.number(&self.environment)?),
                    "setwindow" | "setwindow3" | "setwindow4" => {
                        let advanced = name == "setwindow4";
                        let mut values = [0; 12];
                        for value in values.iter_mut().take(if advanced { 12 } else { 11 }) {
                            *value = args.number(&self.environment)?;
                            args.comma()?;
                        }
                        let color = if args.remaining().starts_with('#') {
                            let input = args.remaining();
                            let end = input.find([',', ' ', '\t']).unwrap_or(input.len());
                            let color = crate::scene::parse_color(&input[..end])?;
                            args.cursor = args.input.len() - input.len() + end;
                            color
                        } else {
                            let name = args.string(&self.environment)?;
                            if !name.starts_with('#') {
                                return Err(Error::invalid(
                                    "image-backed text windows are not implemented",
                                ));
                            }
                            crate::scene::parse_color(&name)?
                        };
                        let mut bounds = [0; 4];
                        for value in &mut bounds {
                            args.comma()?;
                            *value = args.number(&self.environment)?;
                        }
                        let width = i64::from(bounds[2]) - i64::from(bounds[0]);
                        let height = i64::from(bounds[3]) - i64::from(bounds[1]);
                        let window = TextWindow {
                            origin: (values[0], values[1]),
                            bounds: (
                                bounds[0],
                                bounds[1],
                                u32::try_from(width)
                                    .map_err(|_| Error::invalid("invalid text window width"))?,
                                u32::try_from(height)
                                    .map_err(|_| Error::invalid("invalid text window height"))?,
                            ),
                            font_size: u32::try_from(if advanced {
                                values[2]
                            } else {
                                values[4].max(values[5])
                            })
                            .map_err(|_| Error::invalid("negative font size"))?,
                            bold: values[if advanced { 7 } else { 9 }] != 0,
                            shadow: values[10] != 0,
                            color,
                        };
                        if advanced {
                            TextCommand::WindowAdvanced {
                                window,
                                italic: values[8] != 0,
                                underline: values[9] != 0,
                                border: values[11] != 0,
                                spacing: values[4],
                                line_height: if values[5] == -1 {
                                    None
                                } else {
                                    Some(
                                        u32::try_from(values[5])
                                            .map_err(|_| Error::invalid("negative line height"))?,
                                    )
                                },
                                wrap_width: u32::try_from(values[3])
                                    .map_err(|_| Error::invalid("negative wrap width"))?,
                                speed: u8::try_from(values[6])
                                    .map_err(|_| Error::invalid("invalid text speed"))?,
                            }
                        } else {
                            TextCommand::Window(window)
                        }
                    }
                    "setwindow2" => TextCommand::WindowColor(crate::scene::parse_color(
                        &args.string(&self.environment)?,
                    )?),
                    "text_speed" => TextCommand::Speed(
                        u8::try_from(args.number(&self.environment)?)
                            .ok()
                            .filter(|speed| *speed <= 10)
                            .ok_or_else(|| Error::invalid("text_speed must be between 0 and 10"))?,
                    ),
                    "texton" | "textshow" => TextCommand::Visible(true),
                    "textoff" | "texthide" => TextCommand::Visible(false),
                    "textclear" => TextCommand::Clear,
                    "br" => TextCommand::NewLine,
                    _ => return Err(Error::invalid("unknown text command")),
                };
                args.finish()?;
                self.blocked = true;
                return Ok(Some(Event::Text { command, line }));
            }
            Opcode::Effect => {
                if !self.defining {
                    return Err(Error::invalid("effects must be defined before game"));
                }
                let id = args.number(&self.environment)?;
                if id < 2 {
                    return Err(Error::invalid(
                        "effect definition number must be at least 2",
                    ));
                }
                args.comma()?;
                let effect = args.effect(&self.environment, &HashMap::new())?;
                args.finish()?;
                if !self.effects.contains_key(&id) {
                    self.environment.budget(128, self.limits.state_bytes)?;
                }
                self.effects
                    .try_reserve(1)
                    .map_err(|_| Error::Limit("effect allocation"))?;
                self.effects.insert(id, effect);
            }
            Opcode::Scene => {
                let command = match name.as_ref() {
                    "bg" => {
                        let input = args.remaining();
                        let value = if input.starts_with('#') {
                            let end = input.find([',', ' ', '\t']).unwrap_or(input.len());
                            let value = input[..end].to_owned();
                            args.cursor = args.input.len() - input.len() + end;
                            value
                        } else if input
                            .split([',', ' ', '\t'])
                            .next()
                            .is_some_and(|word| matches!(word, "black" | "white"))
                        {
                            args.name()?.to_owned()
                        } else {
                            args.string(&self.environment)?
                        };
                        args.comma()?;
                        SceneCommand::Background {
                            name: value,
                            effect: args.effect(&self.environment, &self.effects)?,
                        }
                    }
                    "lsp" | "lsph" => {
                        let slot = args.nonnegative(&self.environment)?;
                        args.comma()?;
                        let image = args.string(&self.environment)?;
                        args.comma()?;
                        let x = args.number(&self.environment)?;
                        args.comma()?;
                        let y = args.number(&self.environment)?;
                        let opacity = if args.eat(",") {
                            args.number(&self.environment)?.clamp(0, 255) as u8
                        } else {
                            255
                        };
                        SceneCommand::Load {
                            slot,
                            image,
                            x,
                            y,
                            opacity,
                            visible: name == "lsp",
                        }
                    }
                    "csp" => {
                        let first = args.number(&self.environment)?;
                        let last = if args.eat(",") {
                            args.number(&self.environment)?
                        } else {
                            first
                        };
                        SceneCommand::Clear { first, last }
                    }
                    "vsp" => {
                        let slot = args.nonnegative(&self.environment)?;
                        args.comma()?;
                        SceneCommand::Visible {
                            slot,
                            visible: args.number(&self.environment)? != 0,
                        }
                    }
                    "msp" | "amsp" => {
                        let slot = args.nonnegative(&self.environment)?;
                        args.comma()?;
                        let x = args.number(&self.environment)?;
                        args.comma()?;
                        let y = args.number(&self.environment)?;
                        let opacity = if args.eat(",") {
                            Some(args.number(&self.environment)?)
                        } else {
                            None
                        };
                        SceneCommand::Move {
                            slot,
                            x,
                            y,
                            opacity,
                            relative: name == "msp",
                        }
                    }
                    "cell" => {
                        let slot = args.nonnegative(&self.environment)?;
                        args.comma()?;
                        SceneCommand::Cell {
                            slot,
                            cell: args.nonnegative(&self.environment)?,
                        }
                    }
                    "allsphide" => SceneCommand::HideAll(true),
                    "allspresume" => SceneCommand::HideAll(false),
                    "print" => {
                        SceneCommand::Present(args.effect(&self.environment, &self.effects)?)
                    }
                    _ => return Err(Error::invalid("unknown scene command")),
                };
                args.finish()?;
                self.blocked = true;
                return Ok(Some(Event::Scene { command, line }));
            }
            Opcode::Other => {
                return Err(Error::invalid(format!(
                    "command {name} is not implemented in the Rust runtime"
                )));
            }
        }
        Ok(None)
    }
}

fn arithmetic(left: i32, right: i32, operator: u8) -> Result<i32> {
    match operator {
        b'+' => Ok(left.wrapping_add(right)),
        b'-' => Ok(left.wrapping_sub(right)),
        b'*' => Ok(left.wrapping_mul(right)),
        b'/' if right != 0 => Ok(left.wrapping_div(right)),
        b'%' if right != 0 => Ok(left.wrapping_rem(right)),
        _ => Err(Error::invalid("division/remainder by zero")),
    }
}

struct Arguments<'a> {
    input: &'a str,
    cursor: usize,
    limits: Limits,
}

impl<'a> Arguments<'a> {
    fn new(input: &'a str, limits: Limits) -> Self {
        Self {
            input,
            cursor: 0,
            limits,
        }
    }
    fn remaining(&self) -> &'a str {
        self.input[self.cursor..].trim_start()
    }
    fn spaces(&mut self) {
        self.cursor = self.input.len() - self.remaining().len();
    }

    fn eat(&mut self, text: &str) -> bool {
        self.spaces();
        if self
            .remaining()
            .get(..text.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(text))
        {
            self.cursor += text.len();
            true
        } else {
            false
        }
    }

    fn name(&mut self) -> Result<&'a str> {
        self.spaces();
        let start = self.cursor;
        while self
            .input
            .as_bytes()
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            self.cursor += 1;
        }
        if start == self.cursor {
            return Err(Error::invalid("expected a name"));
        }
        Ok(&self.input[start..self.cursor])
    }

    fn comma(&mut self) -> Result<()> {
        if self.eat(",") {
            Ok(())
        } else {
            Err(Error::invalid("expected a comma"))
        }
    }
    fn finish(&self) -> Result<()> {
        if self.remaining().is_empty() {
            Ok(())
        } else {
            Err(Error::invalid("unexpected command arguments"))
        }
    }

    fn index(&mut self, environment: &Environment) -> Result<u32> {
        self.spaces();
        let value = if self
            .remaining()
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit)
        {
            self.name()?
                .parse::<u32>()
                .map_err(|_| Error::invalid("invalid variable index"))?
        } else {
            let name = self.name()?.to_ascii_lowercase();
            let value = environment
                .number_aliases
                .get(&name)
                .copied()
                .ok_or_else(|| Error::invalid(format!("unknown numeric alias {name}")))?;
            u32::try_from(value).map_err(|_| Error::invalid("negative variable index"))?
        };
        if value > i32::MAX as u32 {
            return Err(Error::invalid(
                "variable index exceeds the script integer range",
            ));
        }
        Ok(value)
    }

    fn variable(&mut self, environment: &Environment) -> Result<(bool, u32)> {
        let string = if self.eat("$") {
            true
        } else if self.eat("%") {
            false
        } else {
            return Err(Error::invalid("expected a variable"));
        };
        Ok((string, self.index(environment)?))
    }

    fn number(&mut self, environment: &Environment) -> Result<i32> {
        self.expression(environment, 0, 0)
    }

    fn nonnegative(&mut self, environment: &Environment) -> Result<u32> {
        u32::try_from(self.number(environment)?)
            .map_err(|_| Error::invalid("negative scene parameter"))
    }

    fn next_number(&mut self, environment: &Environment) -> Result<i32> {
        self.comma()?;
        self.number(environment)
    }

    fn next_color(&mut self, environment: &Environment) -> Result<[u8; 3]> {
        self.comma()?;
        if self.remaining().starts_with('#') {
            let input = self.remaining();
            let end = input.find([',', ' ', '\t']).unwrap_or(input.len());
            let color = crate::scene::parse_color(&input[..end])?;
            self.cursor = self.input.len() - input.len() + end;
            Ok(color)
        } else {
            crate::scene::parse_color(&self.string(environment)?)
        }
    }

    fn text_preset(&mut self, environment: &Environment) -> Result<(u32, TextPreset)> {
        fn inherited(value: i32) -> Result<Option<u32>> {
            if value == -1 {
                Ok(None)
            } else {
                u32::try_from(value)
                    .map(Some)
                    .map_err(|_| Error::invalid("invalid text preset parameter"))
            }
        }
        let number = self.nonnegative(environment)?;
        let font = u8::try_from(self.next_number(environment)?)
            .map_err(|_| Error::invalid("invalid font number"))?;
        let size = inherited(self.next_number(environment)?)?;
        let color = self.next_color(environment)?;
        let bold = self.next_number(environment)? != 0;
        let italic = self.next_number(environment)? != 0;
        let underline = self.next_number(environment)? != 0;
        let border = self.next_number(environment)? != 0;
        let border_width = inherited(self.next_number(environment)?)?
            .map(|width| {
                u16::try_from(u64::from(width) * 25).map_err(|_| Error::Limit("text border"))
            })
            .transpose()?;
        let border_color = self.next_color(environment)?;
        let shadow = self.next_number(environment)? != 0;
        let shadow_x = self.next_number(environment)?;
        let shadow_y = self.next_number(environment)?;
        let shadow_color = self.next_color(environment)?;
        let spacing = self.next_number(environment)?;
        let line_height = if self.remaining().starts_with(',') {
            inherited(self.next_number(environment)?)?
        } else {
            None
        };
        let wrap_width = if self.remaining().starts_with(',') {
            inherited(self.next_number(environment)?)?
        } else {
            None
        };
        Ok((
            number,
            TextPreset {
                font,
                size,
                color,
                bold,
                italic,
                underline,
                border,
                border_width,
                border_color,
                shadow,
                shadow_x: (shadow_x != -1).then_some(shadow_x),
                shadow_y: (shadow_y != -1).then_some(shadow_y),
                shadow_color,
                spacing: if spacing == -999 { 0 } else { spacing },
                line_height,
                wrap_width,
            },
        ))
    }

    fn effect(
        &mut self,
        environment: &Environment,
        effects: &HashMap<i32, Effect>,
    ) -> Result<Effect> {
        let id = self.number(environment)?;
        if let Some(effect) = effects.get(&id) {
            return Ok(*effect);
        }
        let duration_ms = match id {
            0 | 1 => 0,
            10 => {
                self.comma()?;
                self.nonnegative(environment)?
            }
            _ => return Err(Error::invalid(format!("effect {id} is not implemented"))),
        };
        Ok(Effect { duration_ms })
    }

    fn expression(&mut self, environment: &Environment, minimum: u8, depth: usize) -> Result<i32> {
        if depth >= self.limits.expression_depth {
            return Err(Error::Limit("expression nesting"));
        }
        self.spaces();
        let mut left = if self.eat("-") {
            self.expression(environment, 5, depth + 1)?.wrapping_neg()
        } else if self.eat("+") {
            self.expression(environment, 5, depth + 1)?
        } else if self.eat("(") {
            let value = self.expression(environment, 0, depth + 1)?;
            if !self.eat(")") {
                return Err(Error::invalid("missing closing parenthesis"));
            }
            value
        } else if self.eat("%") {
            let variable = self.index(environment)?;
            environment.numbers.get(&variable).copied().unwrap_or(0)
        } else {
            let name = self.name()?;
            if name.starts_with("0x") || name.starts_with("0X") {
                u32::from_str_radix(&name[2..], 16)
                    .map(|value| value as i32)
                    .map_err(|_| Error::invalid("invalid hex integer"))?
            } else if name.as_bytes()[0].is_ascii_digit() {
                name.parse::<u32>()
                    .ok()
                    .filter(|&value| value <= 0x8000_0000)
                    .map(|value| value as i32)
                    .ok_or_else(|| Error::invalid("integer outside 32-bit range"))?
            } else {
                environment
                    .number_aliases
                    .get(&name.to_ascii_lowercase())
                    .copied()
                    .ok_or_else(|| Error::invalid(format!("unknown numeric alias {name}")))?
            }
        };
        loop {
            self.spaces();
            let rest = self.remaining();
            let (operator, precedence, width) = match rest.as_bytes().first() {
                Some(b'+') => (b'+', 1, 1),
                Some(b'-') => (b'-', 1, 1),
                Some(b'*') => (b'*', 3, 1),
                Some(b'/') => (b'/', 3, 1),
                _ if rest
                    .get(..3)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("mod"))
                    && rest
                        .as_bytes()
                        .get(3)
                        .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_') =>
                {
                    (b'%', 3, 3)
                }
                _ => break,
            };
            if precedence < minimum {
                break;
            }
            self.cursor += width;
            let right = self.expression(environment, precedence + 1, depth + 1)?;
            left = arithmetic(left, right, operator)?;
        }
        Ok(left)
    }

    fn string(&mut self, environment: &Environment) -> Result<String> {
        let mut result = String::new();
        loop {
            self.spaces();
            let value = if self.eat("\"") || self.eat("`") {
                let delimiter = self.input.as_bytes()[self.cursor - 1] as char;
                let start = self.cursor;
                let count = self.input[self.cursor..]
                    .find(delimiter)
                    .unwrap_or(self.input.len() - self.cursor);
                self.cursor += count;
                let value = &self.input[start..self.cursor];
                if self.cursor < self.input.len() {
                    self.cursor += 1;
                }
                value
            } else if self.eat("$") {
                let variable = self.index(environment)?;
                environment
                    .strings
                    .get(&variable)
                    .map_or("", String::as_str)
            } else if self.eat("*") {
                return Ok(format!("*{}", self.name()?));
            } else {
                let name = self.name()?.to_ascii_lowercase();
                environment
                    .string_aliases
                    .get(&name)
                    .map(String::as_str)
                    .ok_or_else(|| Error::invalid(format!("unknown string alias {name}")))?
            };
            if value.len() > self.limits.state_bytes.saturating_sub(result.len()) {
                return Err(Error::Limit("string expression"));
            }
            result
                .try_reserve(value.len())
                .map_err(|_| Error::Limit("string allocation"))?;
            result.push_str(value);
            if !self.eat("+") {
                break;
            }
        }
        Ok(result)
    }

    fn condition(&mut self, environment: &Environment) -> Result<bool> {
        let possible_alias = self
            .remaining()
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let string_alias = environment.string_aliases.contains_key(&possible_alias)
            && !environment.number_aliases.contains_key(&possible_alias);
        let (operator, ordering) = if string_alias || self.remaining().starts_with(['$', '"', '`'])
        {
            let left = self.string(environment)?;
            (self.comparison()?, left.cmp(&self.string(environment)?))
        } else {
            let left = self.number(environment)?;
            (self.comparison()?, left.cmp(&self.number(environment)?))
        };
        Ok(match operator {
            "=" | "==" => ordering.is_eq(),
            "!=" | "<>" => !ordering.is_eq(),
            "<" => ordering.is_lt(),
            ">" => ordering.is_gt(),
            "<=" => !ordering.is_gt(),
            ">=" => !ordering.is_lt(),
            _ => unreachable!(),
        })
    }

    fn conditions(&mut self, environment: &Environment, positive: bool) -> Result<bool> {
        let mut mode = None;
        let mut any = false;
        for _ in 0..self.limits.expression_depth {
            let matched = self.condition(environment)? == positive;
            any |= matched;
            if self.eat("|") {
                if mode == Some('&') {
                    return Err(Error::invalid(
                        "mixing & and | in a condition is unsupported",
                    ));
                }
                while self.eat("|") {}
                mode = Some('|');
                continue;
            }
            if (mode == Some('|') && !any) || (mode != Some('|') && !matched) {
                return Ok(false);
            }
            if self.eat("&") {
                if mode == Some('|') {
                    return Err(Error::invalid(
                        "mixing & and | in a condition is unsupported",
                    ));
                }
                while self.eat("&") {}
                mode = Some('&');
                continue;
            }
            return Ok(true);
        }
        Err(Error::Limit("conditional clauses"))
    }

    fn comparison(&mut self) -> Result<&'static str> {
        for operator in ["==", "!=", "<=", ">=", "<>", "=", "<", ">"] {
            if self.eat(operator) {
                return Ok(operator);
            }
        }
        Err(Error::invalid("expected a comparison operator"))
    }
}
