
use log::{debug, error, info, trace, warn};
use minifb::{Window, WindowOptions, Key};
use rand::Rng;
use std::fs::File;
use std::io::Read;
use std::time::{Duration, Instant};
use std::cell::RefCell;
use minifb::InputCallback;
use std::rc::Rc;

struct Color(u8, u8, u8);

fn bright(v: u8) -> u8 {
    match v {
        0b0000_0011 => 0b1111_1111,
        0b0000_0010 => 0b0011_1111,
        0b0000_0001 => 0b0000_1111,
        0b0000_0000 => 0b0000_0011,
        _ => unreachable!(),
    }
}

fn unpack_color(reg: u8) -> Color {
    let reg = reg & 0b0011_1111;

    let red: u8 = reg & 0b0000_0011;
    let green: u8 = (reg >> 2) & 0b0000_0011;
    let blue: u8 = (reg >> 4) & 0b0000_0011;

    let red = bright(red);
    let green = bright(green);
    let blue = bright(blue);

    Color(red, green, blue)
}

fn makeRGB(c: &Color) -> u32 {
    let mut color: u32 = 0;
    color |= (c.0 as u32) << 16;   // R
    color |= (c.1 as u32) << 8;    // G
    color |= (c.2 as u32) << 0;    // B
    return color;
}

#[derive(Debug)]
enum Register {
    AC,
    X,
    Y,
    OUT,
}

#[derive(Debug, Clone)]
struct CpuState {
    PC: u16,
    IR: u8,
    D: u8,
    AC: u8,
    X: u8,
    Y: u8,
    OUT: u8,
    undef: u8,
}

struct Gigatron {
    ROM: [[u8; 2]; 1 << 16],
    RAM: [u8; 1 << 15],
    IN: u8,
    S: CpuState,
    video: VGA,
    vgaX: i32,
    vgaY: i32,
    t: i64,
    joy: Option<Direction>,
}

fn E(W: bool, p: Register) -> Option<Register> {
    if W { None } else { Some(p) } // Disable AC and OUT loading during RAM write
}

fn makeAddr(hi: u8, lo: u8) -> u16 {
    (hi as u16) << 8 | lo as u16
}

fn busy_wait(target_duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < target_duration {
        std::hint::spin_loop();
    }
}

type KeyVec = Rc<RefCell<Vec<u32>>>;

#[derive(PartialEq, Debug)]
enum Direction {
    Up,
    Left,
    Right,
    Down,
    ButtonA,
    ButtonB,
    Start,
    Select,
}

struct VGA {
    width: usize,
    height: usize,
    buffer: Vec<u32>,
    window: Window,
    keys: KeyVec,
}

struct Input {
    keys: KeyVec,
}

impl InputCallback for Input {
    fn add_char(&mut self, uni_char: u32) {
        self.keys.borrow_mut().push(uni_char);
    }
}

impl VGA {
    fn new(width: usize, height: usize) -> Self {
        let mut buffer: Vec<u32> = vec![0u32; width * height];
        let mut window = Window::new("Gigatron TTL Simulator (c) Vitold S", width, height, WindowOptions::default()).unwrap();
        let keys = KeyVec::new(RefCell::new(Vec::new()));
        window.set_input_callback(Box::new(Input { keys: keys.clone() }));
        VGA {
            width,
            height,
            buffer,
            window,
            keys,
        }

    }

    fn put(&mut self, vgaX: usize, vgaY: usize, color: u32) {
        if vgaX < self.width && vgaY < self.height {
            let offset: usize = vgaY as usize * self.width + vgaX as usize;
            //self.buffer[offset] = 0xFF00_0000 | 0x00FF_0000; // ARGB (красный)
            self.buffer[offset] = 0xFF00_0000 | color;
        }
    }

    fn update(&mut self) {
        if self.window.is_open() {
            self.window
                .update_with_buffer(&self.buffer, self.width, self.height)
                .unwrap();
        }
    }

    fn check_key(&mut self) -> Option<char> {
        let mut keys = self.keys.borrow_mut();
        let mut key = None;
        for t in keys.iter() {
            key = char::from_u32(*t);
        }
        keys.clear();
        key
    }

    fn check_joystick(&mut self) -> Option<Direction> {
        let mut result: Option<Direction> = None;
        if self.window.is_key_down(Key::Up) {
            result = Some(Direction::Up);
        }
        if self.window.is_key_down(Key::Down) {
            result = Some(Direction::Down);
        }
        if self.window.is_key_down(Key::Left) {
            result = Some(Direction::Left);
        }
        if self.window.is_key_down(Key::Right) {
            result = Some(Direction::Right);
        }
        if self.window.is_key_down(Key::Enter) {
            result = Some(Direction::Start);
        }
        if self.window.is_key_down(Key::Backspace) {
            result = Some(Direction::Select);
        }
        if self.window.is_key_down(Key::Space) {
            result = Some(Direction::ButtonA);
        }
        if self.window.is_key_down(Key::Tab) {
            result = Some(Direction::ButtonB);
        }
        result
    }

}

impl Gigatron {
    pub fn new() -> Self {
        Gigatron {
            ROM: [[0u8; 2]; 1 << 16],
            RAM: [0u8; 1 << 15],
            S: CpuState::new(),
            IN: 0xff,
            video: VGA::new(640, 480),
            vgaX: 0,
            vgaY: 0,
            t: -2,
            joy: None,
        }
    }

    fn garble(&mut self) {
        let mut rng = rand::rng();
        //garble( &RAM );
        rng.fill(&mut self.RAM);
        //garble( &S );
        self.S.PC = rand::random();
        self.S.IR = rand::random();
        self.S.D = rand::random();
        self.S.AC = rand::random();
        self.S.X = rand::random();
        self.S.Y = rand::random();
        self.S.OUT = rand::random();
        self.S.undef = rand::random();
    }

    fn init(&mut self) {
        self.garble();
        self.IN = 0xFF;
    }

    fn read_rom(&mut self, filename: &str) -> std::io::Result<()> {
        let mut file = File::open(filename)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        if buffer.len() != 65536 * 2 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "File size must be exactly 131072 bytes",
            ));
        }
        for (i, chunk) in buffer.chunks_exact(2).enumerate() {
            self.ROM[i] = [chunk[0], chunk[1]];
        }
        Ok(())
    }

    fn read_ram(&mut self) -> std::io::Result<()> {
        //    let mut f = File::create_new("foo.txt")?;
        //    f.write_all("Hello, world!".as_bytes())?;
        Ok(())
    }

    fn write_ram(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn cpuCycle(&mut self) -> CpuState {
        let mut T: CpuState = self.S.clone(); // New state is old state unless something changes
        T.IR = self.ROM[self.S.PC as usize][0]; // Instruction Fetch
        T.D = self.ROM[self.S.PC as usize][1];
        let ins = self.S.IR >> 5; // Instruction
        let mode = (self.S.IR >> 2) & 7; // Addressing mode (or condition)
        let bus = self.S.IR & 3; // Busmode
        let W = ins == 6; // Write instruction?
        let J = ins == 7; // Jump instruction?

        let mut lo = self.S.D;
        let mut hi = 0;
        let mut to: Option<Register> = None; // Mode Decoder
        let mut incX = false;
        if !J {
            match mode {
                0 => {
                    to = E(W, Register::AC);
                }
                1 => {
                    to = E(W, Register::AC);
                    lo = self.S.X;
                }
                2 => {
                    to = E(W, Register::AC);
                    hi = self.S.Y;
                }
                3 => {
                    to = E(W, Register::AC);
                    lo = self.S.X;
                    hi = self.S.Y;
                }
                4 => {
                    to = Some(Register::X);
                }
                5 => {
                    to = Some(Register::Y);
                }
                6 => {
                    to = E(W, Register::OUT);
                }
                7 => {
                    to = E(W, Register::OUT);
                    lo = self.S.X;
                    hi = self.S.Y;
                    incX = true;
                }
                _ => unreachable!(),
            }
        }
        let addr: u16 = makeAddr(hi, lo);
        let mut B = self.S.undef; // Data Bus
        match bus {
            0 => {
                B = self.S.D;
            }
            1 => {
                if !W {
                    let p = addr & 0x7fff;
                    B = self.RAM[p as usize];
                }
            }
            2 => {
                B = self.S.AC;
            }
            3 => {
                B = self.IN;
            }
            _ => unreachable!(),
        }
        if W {
            let p = addr & 0x7fff;
            self.RAM[p as usize] = B; // Random Access Memory
        }
        let mut ALU; // Arithmetic and Logic Unit
        match ins {
            0 => {
                ALU = B; // LD
            }
            1 => {
                ALU = self.S.AC & B; // ANDA
            }
            2 => {
                ALU = self.S.AC | B; // ORA
            }
            3 => {
                ALU = self.S.AC ^ B; // XORA
            }
            4 => {
                ALU = self.S.AC.wrapping_add(B); // ADDA
            }
            5 => {
                ALU = self.S.AC.wrapping_sub(B); // SUBA
            }
            6 => {
                ALU = self.S.AC; // ST
            }
            7 => {
                ALU = self.S.AC.wrapping_neg(); // Bcc/JMP
            }
            _ => unreachable!(),
        };
        if let Some(reg) = to {
            // Load value into register
            //println!("Load value: addr = {:?} value = {}", reg, ALU);
            match reg {
                Register::AC => T.AC = ALU,
                Register::OUT => T.OUT = ALU,
                Register::X => T.X = ALU,
                Register::Y => T.Y = ALU,
            }
            //*to = ALU;
        }
        if incX {
            T.X = self.S.X.wrapping_add(1); // Increment X
        }
        T.PC = self.S.PC.wrapping_add(1); // Next instruction
        if J {
            if mode != 0 {
                // Conditional branch within page
                let sAC = if self.S.AC == 0 { 1 } else { 0 };
                let cond = (self.S.AC >> 7) + 2 * sAC;
                let st = mode & (1 << cond);
                if st > 0 {
                    // 74153
                    T.PC = (self.S.PC & 0xff00) | B as u16;
                }
            } else {
                T.PC = makeAddr(self.S.Y, B); // Unconditional far jump
            }
        }
        T
    }

    fn render(&mut self) {
        for y in 0..120 {
            for x in 0..160 {
                let addr = 2048+y * 256 + x;
                let pixel = self.RAM[addr];
                let color = unpack_color(pixel);
                let rgb = makeRGB(&color);
                self.video.put(2*x as usize, 2*y as usize, rgb);
                self.video.put(2*x as usize + 1, 2*y as usize, rgb);
                self.video.put(2*x as usize, 2*y as usize + 1, rgb);
                self.video.put(2*x as usize + 1, 2*y as usize + 1, rgb);
            }
        }
    }

    fn render2(&mut self) {
        let mut vgaX: usize = 0;
        let mut vgaY: usize = 0;

        let scaleX = self.video.width / 160;
        let scaleY = self.video.height / 120;

        for pixel in self.video.buffer.iter_mut() {

            let x = vgaX / scaleX;
            let y = vgaY / scaleY;

            let addr = 2048+y * 256 + x;
            let v = if addr < 32768 { self.RAM[addr] } else { 0 };
            let color = unpack_color(v);
            let rgb = makeRGB(&color);

            *pixel = rgb;

            vgaX += 1;
            if vgaX == self.video.width {
                vgaX = 0;
                vgaY += 1;
            }
        }
    }

    fn vga(&mut self, T: &mut CpuState) {

        // HSync (бит 2) переключается в 0, когда нужно начать новую строку
        let hSync = ((self.S.OUT & 0b0100_0000) > 0) && ((T.OUT & 0b0100_0000) == 0);

        // VSync (бит 1) переключается в 0, когда нужно начать новый кадр
        let vSync = ((self.S.OUT & 0b1000_0000) > 0) && ((T.OUT & 0b1000_0000) == 0);

        if vSync {
            self.render2();
            self.video.update();
        }

        if hSync {
//            T.undef = rand::random(); // Change this once in a while
        }

        let key = self.video.check_key();
        if let Some(k) = key {
            println!("Character: {:?}", key);
            let ascii = u8::try_from(k as u32);
            match ascii {
                Ok(code) => {
                    self.RAM[0x000f] = code;
                    self.RAM[0x0010] = 0;
                },
                _ => {},
            }
        }

        self.process_joystick();
    }

    fn process_joystick(&mut self) {
        let joy = self.video.check_joystick();
        if let Some(j) = joy {
            if let Some(pj) = &self.joy {
                if *pj == j {
                    return;
                }
            }

            //println!("joystick = {:?}", j);
            
            match j {
                Direction::Right => { self.RAM[0x0011] = 0b1111_1110; self.RAM[0x0010] = 0; } // Bit 0
                Direction::Left => { self.RAM[0x0011] = 0b1111_1101; self.RAM[0x0010] = 0; } // Bit 1
                Direction::Down => { self.RAM[0x0011] = 0b1111_1011; self.RAM[0x0010] = 0; } // Bit 2
                Direction::Up => { self.RAM[0x0011] = 0b1111_0111; self.RAM[0x0010] = 0; } // Bit 3
                Direction::Start => { self.RAM[0x0011] = 0b1110_1111; self.RAM[0x0010] = 0; }, // Bit 4
                Direction::Select => { self.RAM[0x0011] = 0b1101_1111; self.RAM[0x0010] = 0; }, // Bit 5
                Direction::ButtonB => { self.RAM[0x0011] = 0b1011_1111; self.RAM[0x0010] = 0; }, // Bit 6
                Direction::ButtonA => { self.RAM[0x0011] = 0b0111_1111; self.RAM[0x0010] = 0; }, // Bit 7
            }
            
            self.joy = Some(j);
        }
    }

    fn run(&mut self) {
        let delay = Duration::from_nanos(160);

        loop {
            if self.t < 0 {
                self.S.PC = 0; // MCP100 Power-On Reset
            }
            let mut T: CpuState = self.cpuCycle(); // Update CPU
            self.vga(&mut T);
            self.S = T;
            self.t += 1;
            //busy_wait(delay);
        }
    }
}

impl CpuState {
    pub fn new() -> Self {
        CpuState {
            PC: 0,
            IR: 0,
            D: 0,
            AC: 0,
            X: 0,
            Y: 0,
            OUT: 0,
            undef: 0,
        }
    }
}

fn main() {
    let mut E: Gigatron = Gigatron::new();
    E.init();
//    E.read_rom("ROMv1.rom").expect("No ROM.");
    E.read_rom("ROMv6.rom").expect("No ROM.");
    E.run();
}
