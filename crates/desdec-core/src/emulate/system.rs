//! Requests made to an operating system, observed without answering them.
//!
//! This module is deliberately a decoder, not a syscall implementation.  The
//! emulator never forwards a request to the host and never invents its return
//! value.  It records the ABI state at the boundary, which is the useful part
//! of a system-call tracer when studying a binary safely.

use iced_x86::Register;

use crate::{
    Architecture, BinaryFormat,
    emulate::{registers::Registers, syscalls},
};

/// The ABI used by the request that stopped the emulation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemPlatform {
    /// Linux's x86-64 syscall ABI, the convention shown by `strace`.
    LinuxX86_64,
    /// Linux's aarch64 (ARM64) `svc #0` ABI: the number is in `x8`, the
    /// arguments in `x0`–`x5`.
    LinuxArm64,
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
            Self::LinuxArm64 => "Linux ARM64",
            Self::LinuxX86 => "Linux x86",
            Self::MacOsX86_64 => "macOS x86-64",
            Self::WindowsX86_64 => "Windows x86-64",
            Self::Unknown => "unknown ABI",
        }
    }

    /// Chooses the ABI from the architecture and container. The architecture
    /// decides first: an ELF may be x86-64 or aarch64, and only the machine
    /// type tells them apart — their syscall numbers do not agree.
    #[must_use]
    const fn identify(architecture: Architecture, format: BinaryFormat, bitness: u32) -> Self {
        if matches!(architecture, Architecture::Arm64) {
            // The interpreter is x86-only today, so a `svc #0` is reached only
            // once an aarch64 interpreter exists; the ABI is decoded now so
            // the number resolves against the right table when it does.
            return match format {
                BinaryFormat::Elf { .. } => Self::LinuxArm64,
                _ => Self::Unknown,
            };
        }
        match (format, bitness) {
            (BinaryFormat::Elf { .. }, 64) => Self::LinuxX86_64,
            (BinaryFormat::Elf { .. }, 32) => Self::LinuxX86,
            (BinaryFormat::MachO { .. }, 64) => Self::MacOsX86_64,
            (BinaryFormat::Pe, 64) => Self::WindowsX86_64,
            _ => Self::Unknown,
        }
    }

    /// Reads the service number and the six argument registers this ABI uses.
    fn capture_registers(self, registers: &Registers) -> (u64, [SystemArgument; 6]) {
        if matches!(self, Self::LinuxArm64) {
            // aarch64: the number is in `x8`, the arguments in `x0`–`x5`,
            // reached by slot because this register file has no aarch64 names.
            let arguments = [0, 1, 2, 3, 4, 5].map(|index| SystemArgument {
                register: AARCH64_ARGUMENTS[index],
                value: registers.slot(index),
            });
            return (registers.slot(8), arguments);
        }
        let (number_register, argument_registers) = match self {
            Self::LinuxX86 => (
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
            // Every other 64-bit ABI here numbers the syscall in `rax` and
            // passes arguments in the System V order with `r10` for the fourth.
            _ => (
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
        let arguments = argument_registers.map(|(register, source)| SystemArgument {
            register,
            value: registers.get(source),
        });
        (registers.get(number_register.1), arguments)
    }
}

/// The aarch64 argument registers, in call order.
const AARCH64_ARGUMENTS: [&str; 6] = ["x0", "x1", "x2", "x3", "x4", "x5"];

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
    /// Identifies the ABI from the architecture and binary container, and
    /// captures its register state.  Nothing is executed and no argument
    /// pointer is read.
    #[must_use]
    pub fn capture(
        architecture: Architecture,
        format: BinaryFormat,
        bitness: u32,
        registers: &Registers,
    ) -> Self {
        let platform = SystemPlatform::identify(architecture, format, bitness);
        let (number, arguments) = platform.capture_registers(registers);
        Self {
            platform,
            number,
            name: name(platform, number),
            arguments,
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
        SystemPlatform::LinuxX86_64 => syscalls::linux_x86_64(number),
        SystemPlatform::LinuxArm64 => syscalls::linux_arm64(number),
        SystemPlatform::LinuxX86 => syscalls::linux_x86(number),
        // macOS tags each syscall's class in the high bits (the UNIX/BSD
        // class is `0x0200_0000`); the table is keyed by the plain ordinal.
        SystemPlatform::MacOsX86_64 => syscalls::macos_x86_64(number & 0x00ff_ffff),
        // NT service IDs are deliberately not decoded: Microsoft changes them
        // between releases, so a fixed table would confidently lie.
        SystemPlatform::WindowsX86_64 | SystemPlatform::Unknown => None,
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
            Architecture::X86_64,
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
        let call = SystemCall::capture(Architecture::X86_64, BinaryFormat::Pe, 64, &registers);
        assert_eq!(call.platform, SystemPlatform::WindowsX86_64);
        assert_eq!(call.name, None);
        assert_eq!(call.display_name(), "syscall_0x55");
    }

    #[test]
    fn aarch64_reads_x8_and_names_it_against_the_arm_table() {
        // The interpreter has no aarch64 register names, so the ABI reaches
        // `x8`/`x0`… by slot; slot 8 is `r8` under the x86-64 file's naming.
        let mut registers = Registers::new();
        registers.set(Register::R8, 221); // x8: the service number
        registers.set(Register::RAX, 42); // x0: the first argument
        let call = SystemCall::capture(
            Architecture::Arm64,
            BinaryFormat::Elf {
                bits: 64,
                endianness: crate::Endianness::Little,
            },
            64,
            &registers,
        );
        assert_eq!(call.platform, SystemPlatform::LinuxArm64);
        assert_eq!(call.number, 221);
        // 221 is `execve` on aarch64, not the `fadvise64` it is on x86-64.
        assert_eq!(call.name, Some("execve"));
        assert_eq!(
            call.arguments[0],
            SystemArgument {
                register: "x0",
                value: 42
            }
        );
    }
}
