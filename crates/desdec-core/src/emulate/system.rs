//! Requests made to an operating system, observed without answering them.
//!
//! This module is deliberately a decoder, not a syscall implementation.  The
//! emulator never forwards a request to the host and never invents its return
//! value.  It records the ABI state at the boundary, which is the useful part
//! of a system-call tracer when studying a binary safely.

use iced_x86::Register;

use crate::{BinaryFormat, emulate::registers::Registers};

/// The ABI used by the request that stopped the emulation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemPlatform {
    /// Linux's x86-64 syscall ABI, the convention shown by `strace`.
    LinuxX86_64,
    /// Linux's 32-bit `int 0x80` ABI.
    LinuxX86,
    /// macOS's x86-64 BSD/Mach syscall ABI, as observed by `dtruss`.
    MacOsX86_64,
    /// Windows's native syscall ABI. The service number changes by build.
    WindowsX86_64,
    /// A binary whose ABI cannot be known from its container.
    Unknown,
}

impl SystemPlatform {
    /// A short, deliberately factual name for the UI and exports.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "Linux x86-64",
            Self::LinuxX86 => "Linux x86",
            Self::MacOsX86_64 => "macOS x86-64",
            Self::WindowsX86_64 => "Windows x86-64",
            Self::Unknown => "unknown ABI",
        }
    }
}

/// One argument as the ABI presents it.  Pointer values are not dereferenced:
/// a pointer may be invalid, and displaying a fabricated string would be as
/// misleading as fabricating a syscall result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemArgument {
    pub register: &'static str,
    pub value: u64,
}

/// A syscall boundary observed in the emulated processor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemCall {
    pub platform: SystemPlatform,
    /// The raw service number in the ABI register.
    pub number: u64,
    /// A name only where it is stable enough to be honest.
    pub name: Option<&'static str>,
    pub arguments: [SystemArgument; 6],
}

impl SystemCall {
    /// Identifies the ABI from the binary container and captures its register
    /// state.  Nothing is executed and no argument pointer is read.
    #[must_use]
    pub fn capture(format: BinaryFormat, bitness: u32, registers: &Registers) -> Self {
        let platform = match (format, bitness) {
            (BinaryFormat::Elf { .. }, 64) => SystemPlatform::LinuxX86_64,
            (BinaryFormat::Elf { .. }, 32) => SystemPlatform::LinuxX86,
            (BinaryFormat::MachO { .. }, 64) => SystemPlatform::MacOsX86_64,
            (BinaryFormat::Pe, 64) => SystemPlatform::WindowsX86_64,
            _ => SystemPlatform::Unknown,
        };
        let (number_register, argument_registers) = match platform {
            SystemPlatform::LinuxX86_64
            | SystemPlatform::MacOsX86_64
            | SystemPlatform::WindowsX86_64 => (
                ("rax", Register::RAX),
                [
                    ("rdi", Register::RDI),
                    ("rsi", Register::RSI),
                    ("rdx", Register::RDX),
                    ("r10", Register::R10),
                    ("r8", Register::R8),
                    ("r9", Register::R9),
                ],
            ),
            SystemPlatform::LinuxX86 => (
                ("eax", Register::EAX),
                [
                    ("ebx", Register::EBX),
                    ("ecx", Register::ECX),
                    ("edx", Register::EDX),
                    ("esi", Register::ESI),
                    ("edi", Register::EDI),
                    ("ebp", Register::EBP),
                ],
            ),
            SystemPlatform::Unknown => (
                ("rax", Register::RAX),
                [
                    ("rdi", Register::RDI),
                    ("rsi", Register::RSI),
                    ("rdx", Register::RDX),
                    ("r10", Register::R10),
                    ("r8", Register::R8),
                    ("r9", Register::R9),
                ],
            ),
        };
        let number = registers.get(number_register.1);
        Self {
            platform,
            number,
            name: name(platform, number),
            arguments: argument_registers.map(|(register, value)| SystemArgument {
                register,
                value: registers.get(value),
            }),
        }
    }

    /// A compact `strace`-like heading, without claiming a result exists.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.name
            .map_or_else(|| format!("syscall_{:#x}", self.number), str::to_owned)
    }
}

const fn name(platform: SystemPlatform, number: u64) -> Option<&'static str> {
    match platform {
        SystemPlatform::LinuxX86_64 => linux_x86_64(number),
        SystemPlatform::LinuxX86 => linux_x86(number),
        SystemPlatform::MacOsX86_64 => macos_x86_64(number),
        // NT service IDs are deliberately not decoded: Microsoft changes them
        // between releases, so a fixed table would confidently lie.
        SystemPlatform::WindowsX86_64 | SystemPlatform::Unknown => None,
    }
}

const fn linux_x86_64(number: u64) -> Option<&'static str> {
    match number {
        0 => Some("read"),
        1 => Some("write"),
        2 => Some("open"),
        3 => Some("close"),
        9 => Some("mmap"),
        10 => Some("mprotect"),
        11 => Some("munmap"),
        12 => Some("brk"),
        39 => Some("getpid"),
        56 => Some("clone"),
        57 => Some("fork"),
        59 => Some("execve"),
        60 => Some("exit"),
        61 => Some("wait4"),
        72 => Some("fcntl"),
        80 => Some("chdir"),
        89 => Some("readlink"),
        158 => Some("arch_prctl"),
        202 => Some("futex"),
        217 => Some("getdents64"),
        231 => Some("exit_group"),
        257 => Some("openat"),
        262 => Some("newfstatat"),
        273 => Some("set_robust_list"),
        318 => Some("getrandom"),
        334 => Some("rseq"),
        435 => Some("clone3"),
        _ => None,
    }
}

const fn linux_x86(number: u64) -> Option<&'static str> {
    match number {
        1 => Some("exit"),
        2 => Some("fork"),
        3 => Some("read"),
        4 => Some("write"),
        5 => Some("open"),
        6 => Some("close"),
        11 => Some("execve"),
        20 => Some("getpid"),
        45 => Some("brk"),
        54 => Some("ioctl"),
        90 => Some("mmap"),
        91 => Some("munmap"),
        120 => Some("clone"),
        125 => Some("mprotect"),
        252 => Some("exit_group"),
        295 => Some("openat"),
        _ => None,
    }
}

const fn macos_x86_64(number: u64) -> Option<&'static str> {
    // macOS prefixes BSD calls with the UNIX class (0x0200_0000).
    let bsd = number & 0x00ff_ffff;
    match bsd {
        1 => Some("exit"),
        3 => Some("read"),
        4 => Some("write"),
        5 => Some("open"),
        6 => Some("close"),
        20 => Some("getpid"),
        73 => Some("munmap"),
        74 => Some("mprotect"),
        197 => Some("mmap"),
        240 => Some("nanosleep"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_x86_64_decodes_the_number_and_abi_registers() {
        let mut registers = Registers::new();
        registers.set(Register::RAX, 1);
        registers.set(Register::RDI, 2);
        registers.set(Register::RSI, 0x401000);
        registers.set(Register::RDX, 7);
        let call = SystemCall::capture(
            BinaryFormat::Elf {
                bits: 64,
                endianness: crate::Endianness::Little,
            },
            64,
            &registers,
        );
        assert_eq!(call.platform, SystemPlatform::LinuxX86_64);
        assert_eq!(call.name, Some("write"));
        assert_eq!(
            call.arguments[0],
            SystemArgument {
                register: "rdi",
                value: 2
            }
        );
        assert_eq!(
            call.arguments[1],
            SystemArgument {
                register: "rsi",
                value: 0x401000
            }
        );
    }

    #[test]
    fn windows_keeps_the_number_without_a_false_stable_name() {
        let mut registers = Registers::new();
        registers.set(Register::RAX, 0x55);
        let call = SystemCall::capture(BinaryFormat::Pe, 64, &registers);
        assert_eq!(call.platform, SystemPlatform::WindowsX86_64);
        assert_eq!(call.name, None);
        assert_eq!(call.display_name(), "syscall_0x55");
    }
}
