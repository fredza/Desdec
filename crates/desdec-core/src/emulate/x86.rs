//! Carrying out one decoded x86 instruction.
//!
//! The interpreter works on the instruction iced-x86 has already decoded — the
//! same decoder the listing uses, so what runs here is what the reader is
//! looking at, never a second opinion about what the bytes mean.
//!
//! Two decisions shape the whole module:
//!
//! - **An instruction that is not implemented is refused by name, and stops
//!   the run.** It is never approximated and never skipped. A run that quietly
//!   stepped over an `aesenc` would carry on with registers that no execution
//!   would produce, and every value shown afterwards would be a fiction.
//! - **Anything that asks the operating system a question stops the run too.**
//!   There is no operating system here: a `syscall` has nothing to return, and
//!   a made-up return value is the same fiction by another route.
//!
//! The flags are computed in one place per shape of arithmetic rather than at
//! each instruction, because that is where the mistakes hide: `inc` leaves the
//! carry alone and `add` does not, `cmp` is a `sub` that discards its result,
//! and a shift by zero changes nothing at all — not even a flag.

// An interpreter's work *is* changing the width of a value: an eight-bit
// operand read into a sixty-four bit register, a shift count masked to six
// bits, a product held in twice the width it will be stored at. Every
// conversion below is deliberate and is the instruction's own rule, so the
// lints that ask "did you mean to narrow this?" would fire on nearly every
// line and hide the ones worth reading. The narrowing is done in two places
// only — `truncate` and `sign_extend` — and both are tested.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    reason = "changing the width of a value is what these instructions do"
)]

use iced_x86::{Code, ConditionCode, FlowControl, Instruction, Mnemonic, OpKind, Register};

use crate::emulate::{
    memory::{Fault, Memory},
    registers::{Flag, Registers},
};

/// Why carrying out an instruction did not finish.
///
/// Not an error in the emulator: each variant is a fact about the program that
/// the reader is entitled to be told plainly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The instruction is decoded and understood, and this interpreter does
    /// not carry it out. Carries the listing's own text for it.
    Unsupported { text: String },
    /// The program asked the operating system for something.
    SystemCall { text: String },
    /// A memory access could not be carried out.
    Fault(Fault),
    /// A division whose quotient does not exist, or does not fit.
    DivideError,
    /// The program stopped itself: `hlt`, `ud2`.
    Halted { text: String },
}

/// What the instruction did to the flow, once it has been carried out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The instruction pointer is where it should be; carry on.
    Continued,
    /// A call was made, and this is where it will come back to.
    Called { returns_to: u64 },
    /// A call returned.
    Returned,
}

/// The processor state one instruction acts on.
pub struct Cpu<'a> {
    pub registers: &'a mut Registers,
    pub memory: &'a mut Memory,
    /// Whether the code being run is sixty-four bit. It decides the width of a
    /// push, of a return address, and of the default operand size.
    pub bitness: u32,
    /// How many instructions this run has retired, which is the only clock
    /// this machine has. See [`Cpu::rdtsc`].
    pub retired: u64,
    /// Where a branch read its target from, when it read one from memory.
    ///
    /// Set as the branch is carried out rather than worked out afterwards: the
    /// address it read is the one its registers held then, and a call has
    /// moved the stack pointer by the time anything could ask again. It is
    /// what names the call, since a slot no loader has filled in holds zero
    /// and the address branched to says nothing at all.
    pub branched_through: Option<u64>,
}

impl Cpu<'_> {
    /// How many bytes one push or pop moves the stack pointer by.
    const fn word(&self) -> u64 {
        if self.bitness == 64 { 8 } else { 4 }
    }

    /// Carries out one instruction, leaving the instruction pointer where the
    /// next one begins.
    ///
    /// The pointer is advanced *before* the instruction runs, because that is
    /// what the architecture does: a `call` pushes the address of what follows
    /// it, and a RIP-relative operand is relative to what follows it too.
    pub fn execute(&mut self, instruction: &Instruction, text: &str) -> Result<Outcome, Refusal> {
        let next = instruction.next_ip();
        self.registers.instruction_pointer = next;
        // Two families are recognised before the mnemonic is looked at, because
        // the decoder does not give them one name each: every condition has its
        // own mnemonic (`je`, `jne`, `cmovg`, `setb`), and the string
        // instructions carry their width in theirs (`movsb`, `movsq`).
        if let Some(operation) = string_operation_of(instruction.code()) {
            return self.string_operation(instruction, operation, text);
        }
        if instruction.condition_code() != ConditionCode::None {
            return match instruction.flow_control() {
                FlowControl::ConditionalBranch => self.jcc(instruction, text),
                // `setcc` writes one byte and takes nothing; `cmovcc` takes a
                // source as well. Their operand counts tell them apart.
                _ if instruction.op_count() == 1 => self.setcc(instruction),
                _ if instruction.op_count() == 2 => self.cmovcc(instruction),
                _ => Err(Refusal::Unsupported { text: text.into() }),
            };
        }
        match instruction.mnemonic() {
            Mnemonic::Nop | Mnemonic::Endbr64 | Mnemonic::Endbr32 | Mnemonic::Pause => {
                Ok(Outcome::Continued)
            }
            Mnemonic::Mov => self.mov(instruction),
            // The compiler's ordinary 128-bit copy sequence. The aligned and
            // unaligned spellings have the same state effect; alignment is a
            // performance concern on current x86, not a value to invent here.
            Mnemonic::Movaps | Mnemonic::Movups | Mnemonic::Movdqa | Mnemonic::Movdqu => {
                self.vector_move(instruction)
            }
            // These are integer/float names for exactly the same bitwise XOR
            // on an XMM register. Neither changes rflags.
            Mnemonic::Pxor | Mnemonic::Xorps => self.vector_xor(instruction),
            // The narrow moves in and out of a vector register. Small, and the
            // ones a compiler reaches for constantly: glibc's own `strlen`
            // starts with a `movd`, so a statically linked program stopped at
            // its first string operation without them.
            Mnemonic::Movd => self.narrow_vector_move(instruction, 4),
            Mnemonic::Movq => self.narrow_vector_move(instruction, 8),
            // The emulator holds only XMM's low 128 bits, so their values are
            // already what this AVX housekeeping instruction leaves behind.
            Mnemonic::Vzeroupper => Ok(Outcome::Continued),
            Mnemonic::Movzx => self.extend(instruction, false),
            Mnemonic::Movsx | Mnemonic::Movsxd => self.extend(instruction, true),
            Mnemonic::Lea => self.lea(instruction),
            Mnemonic::Xchg => self.xchg(instruction),
            Mnemonic::Push => self.push_operand(instruction),
            Mnemonic::Pop => self.pop_operand(instruction),
            Mnemonic::Add => self.arithmetic(instruction, Arithmetic::Add),
            Mnemonic::Sub => self.arithmetic(instruction, Arithmetic::Sub),
            Mnemonic::Adc => self.arithmetic(instruction, Arithmetic::AddCarry),
            Mnemonic::Sbb => self.arithmetic(instruction, Arithmetic::SubBorrow),
            Mnemonic::Cmp => self.arithmetic(instruction, Arithmetic::Compare),
            Mnemonic::And => self.arithmetic(instruction, Arithmetic::And),
            Mnemonic::Or => self.arithmetic(instruction, Arithmetic::Or),
            Mnemonic::Xor => self.arithmetic(instruction, Arithmetic::Xor),
            Mnemonic::Test => self.arithmetic(instruction, Arithmetic::Test),
            Mnemonic::Inc => self.step_by_one(instruction, true),
            Mnemonic::Dec => self.step_by_one(instruction, false),
            Mnemonic::Neg => self.neg(instruction),
            Mnemonic::Not => self.not(instruction),
            Mnemonic::Imul => self.imul(instruction),
            Mnemonic::Mul => self.mul(instruction),
            Mnemonic::Div => self.divide(instruction, false),
            Mnemonic::Idiv => self.divide(instruction, true),
            Mnemonic::Shl | Mnemonic::Sal => self.shift(instruction, Shift::Left),
            Mnemonic::Shr => self.shift(instruction, Shift::Right),
            Mnemonic::Sar => self.shift(instruction, Shift::Arithmetic),
            Mnemonic::Rol => self.shift(instruction, Shift::RotateLeft),
            Mnemonic::Ror => self.shift(instruction, Shift::RotateRight),
            Mnemonic::Bt | Mnemonic::Bts | Mnemonic::Btr | Mnemonic::Btc => self.bit(instruction),
            Mnemonic::Bsf | Mnemonic::Bsr | Mnemonic::Tzcnt | Mnemonic::Lzcnt => {
                self.bit_scan(instruction)
            }
            Mnemonic::Popcnt => self.popcnt(instruction),
            Mnemonic::Bswap => self.bswap(instruction),
            Mnemonic::Cbw | Mnemonic::Cwde | Mnemonic::Cdqe => {
                Ok(self.sign_extend_accumulator(instruction))
            }
            Mnemonic::Cwd | Mnemonic::Cdq | Mnemonic::Cqo => {
                Ok(self.sign_extend_into_dx(instruction))
            }
            Mnemonic::Jmp => self.jump(instruction, text),
            Mnemonic::Call => self.call(instruction, text),
            Mnemonic::Ret => self.ret(instruction),
            Mnemonic::Leave => self.leave(),
            Mnemonic::Cmpxchg => self.cmpxchg(instruction),
            Mnemonic::Xadd => self.xadd(instruction),
            Mnemonic::Cld => {
                self.registers.set_flag(Flag::Direction, false);
                Ok(Outcome::Continued)
            }
            Mnemonic::Std => {
                self.registers.set_flag(Flag::Direction, true);
                Ok(Outcome::Continued)
            }
            Mnemonic::Clc => {
                self.registers.set_flag(Flag::Carry, false);
                Ok(Outcome::Continued)
            }
            Mnemonic::Stc => {
                self.registers.set_flag(Flag::Carry, true);
                Ok(Outcome::Continued)
            }
            Mnemonic::Cmc => {
                let carry = self.registers.flag(Flag::Carry);
                self.registers.set_flag(Flag::Carry, !carry);
                Ok(Outcome::Continued)
            }
            Mnemonic::Hlt | Mnemonic::Ud0 | Mnemonic::Ud1 | Mnemonic::Ud2 | Mnemonic::Int3 => {
                Err(Refusal::Halted { text: text.into() })
            }
            // A question for the operating system, and there is none here.
            Mnemonic::Syscall | Mnemonic::Sysenter | Mnemonic::Int => {
                Err(Refusal::SystemCall { text: text.into() })
            }
            // A question for the *processor*, which there very much is one of:
            // the one this module builds. These used to be refused alongside
            // the system calls, and the refusal then read `rax` as a system
            // call number and named it — so a `cpuid` came back as a `read`,
            // which is not a smaller answer than the truth but a different
            // one. Worse, glibc asks `cpuid` within the first three hundred
            // instructions of every statically linked program, so every such
            // binary stopped there and could not be run at all.
            Mnemonic::Cpuid => self.cpuid(),
            Mnemonic::Rdtsc | Mnemonic::Rdtscp => self.rdtsc(instruction),
            Mnemonic::Xgetbv => self.xgetbv(),
            // Port access, which is privileged: a user-space program does not
            // reach these, and a run that does is not one to guess past.
            Mnemonic::In | Mnemonic::Out => Err(Refusal::Unsupported { text: text.into() }),
            _ => Err(Refusal::Unsupported { text: text.into() }),
        }
    }

    // ----- operands ---------------------------------------------------------

    /// The address a memory operand designates.
    ///
    /// Segment registers are treated as flat, which is what they are in
    /// sixty-four bit user code, save for `fs` and `gs`. An access through one
    /// of those is refused by the caller rather than answered here with an
    /// address the emulator would have made up.
    fn effective_address(&self, instruction: &Instruction, operand: u32) -> u64 {
        // A RIP-relative operand's displacement is already the address it
        // designates: the decoder has added the instruction's length for us,
        // and adding it a second time would land a page or so past the target.
        if instruction.is_ip_rel_memory_operand() {
            return instruction.memory_displacement64();
        }
        let mut address = instruction.memory_displacement64();
        let base = instruction.memory_base();
        if base != Register::None {
            address = address.wrapping_add(self.registers.get(base));
        }
        let index = instruction.memory_index();
        if index != Register::None {
            let scale = u64::from(instruction.memory_index_scale());
            address = address.wrapping_add(self.registers.get(index).wrapping_mul(scale));
        }
        let _ = operand;
        // A thirty-two bit address wraps at four gigabytes, and an address
        // that wrapped is not the same address with the top bits kept.
        if instruction.memory_size().size() > 0 && self.address_size(instruction) == 4 {
            address &= 0xffff_ffff;
        }
        address
    }

    /// How wide the addresses of this instruction are.
    fn address_size(&self, instruction: &Instruction) -> u32 {
        let base = instruction.memory_base();
        let index = instruction.memory_index();
        if base.is_gpr32() || index.is_gpr32() {
            4
        } else if base.is_gpr16() || index.is_gpr16() {
            2
        } else if self.bitness == 64 {
            8
        } else {
            4
        }
    }

    /// Whether the instruction reaches memory through `fs` or `gs`, which
    /// point at per-thread data no file describes.
    fn uses_thread_segment(instruction: &Instruction) -> bool {
        matches!(instruction.segment_prefix(), Register::FS | Register::GS)
            || matches!(instruction.memory_segment(), Register::FS | Register::GS)
    }

    /// How many bytes one operand of the instruction is.
    fn operand_size(instruction: &Instruction, operand: u32) -> usize {
        match instruction.op_kind(operand) {
            OpKind::Register => instruction.op_register(operand).size(),
            OpKind::Memory => instruction.memory_size().size(),
            _ => instruction.memory_size().size().max(usize::from(
                instruction.op_kind(operand) == OpKind::Immediate8,
            )),
        }
    }

    /// Reads one operand, whatever kind it is.
    fn read_operand(
        &self,
        instruction: &Instruction,
        operand: u32,
        size: usize,
    ) -> Result<u64, Refusal> {
        match instruction.op_kind(operand) {
            OpKind::Register => Ok(self.registers.get(instruction.op_register(operand))),
            OpKind::Memory => {
                if Self::uses_thread_segment(instruction) {
                    return Err(Refusal::Unsupported {
                        text: String::from("fs/gs"),
                    });
                }
                let address = self.effective_address(instruction, operand);
                self.load(address, size)
            }
            OpKind::Immediate8
            | OpKind::Immediate8_2nd
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64 => Ok(truncate(instruction.immediate(operand), size)),
            OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
                Ok(instruction.near_branch_target())
            }
            _ => Err(Refusal::Unsupported {
                text: String::from("operand"),
            }),
        }
    }

    /// Writes one operand, whatever kind it is.
    fn write_operand(
        &mut self,
        instruction: &Instruction,
        operand: u32,
        size: usize,
        value: u64,
    ) -> Result<(), Refusal> {
        match instruction.op_kind(operand) {
            OpKind::Register => {
                self.registers
                    .set(instruction.op_register(operand), truncate(value, size));
                Ok(())
            }
            OpKind::Memory => {
                if Self::uses_thread_segment(instruction) {
                    return Err(Refusal::Unsupported {
                        text: String::from("fs/gs"),
                    });
                }
                let address = self.effective_address(instruction, operand);
                self.store(address, size, value)
            }
            _ => Err(Refusal::Unsupported {
                text: String::from("destination"),
            }),
        }
    }

    /// Reads `size` bytes of memory as one little-endian value.
    fn load(&self, address: u64, size: usize) -> Result<u64, Refusal> {
        let mut bytes = [0_u8; 8];
        let width = size.clamp(1, 8);
        self.memory
            .read(address, &mut bytes[..width])
            .map_err(Refusal::Fault)?;
        Ok(u64::from_le_bytes(bytes))
    }

    /// Writes `size` bytes of memory from one little-endian value.
    fn store(&mut self, address: u64, size: usize, value: u64) -> Result<(), Refusal> {
        let width = size.clamp(1, 8);
        let bytes = value.to_le_bytes();
        self.memory
            .write(address, &bytes[..width])
            .map_err(Refusal::Fault)
    }

    // ----- the stack --------------------------------------------------------

    /// Puts a value on the stack and moves the pointer down.
    pub fn push(&mut self, value: u64) -> Result<(), Refusal> {
        let word = self.word();
        let top = self.registers.stack_pointer().wrapping_sub(word);
        self.store(top, word as usize, value)?;
        self.registers.set_stack_pointer(top);
        Ok(())
    }

    /// Takes a value off the stack and moves the pointer up.
    pub fn pop(&mut self) -> Result<u64, Refusal> {
        let word = self.word();
        let top = self.registers.stack_pointer();
        let value = self.load(top, word as usize)?;
        self.registers.set_stack_pointer(top.wrapping_add(word));
        Ok(value)
    }

    // ----- movement ---------------------------------------------------------

    fn mov(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let value = self.read_operand(instruction, 1, size)?;
        self.write_operand(instruction, 0, size, value)?;
        Ok(Outcome::Continued)
    }

    /// Reads an XMM register or sixteen bytes of ordinary memory.
    fn read_vector_operand(
        &self,
        instruction: &Instruction,
        operand: u32,
    ) -> Result<u128, Refusal> {
        match instruction.op_kind(operand) {
            OpKind::Register => self
                .registers
                .xmm(instruction.op_register(operand))
                .ok_or_else(|| Refusal::Unsupported {
                    text: String::from("vector register"),
                }),
            OpKind::Memory => {
                if Self::uses_thread_segment(instruction) {
                    return Err(Refusal::Unsupported {
                        text: String::from("fs/gs"),
                    });
                }
                let address = self.effective_address(instruction, operand);
                let mut bytes = [0_u8; 16];
                self.memory
                    .read(address, &mut bytes)
                    .map_err(Refusal::Fault)?;
                Ok(u128::from_le_bytes(bytes))
            }
            _ => Err(Refusal::Unsupported {
                text: String::from("vector operand"),
            }),
        }
    }

    /// Writes an XMM register or sixteen bytes of ordinary memory.
    fn write_vector_operand(
        &mut self,
        instruction: &Instruction,
        operand: u32,
        value: u128,
    ) -> Result<(), Refusal> {
        match instruction.op_kind(operand) {
            OpKind::Register => self
                .registers
                .set_xmm(instruction.op_register(operand), value)
                .then_some(())
                .ok_or_else(|| Refusal::Unsupported {
                    text: String::from("vector register"),
                }),
            OpKind::Memory => {
                if Self::uses_thread_segment(instruction) {
                    return Err(Refusal::Unsupported {
                        text: String::from("fs/gs"),
                    });
                }
                let address = self.effective_address(instruction, operand);
                self.memory
                    .write(address, &value.to_le_bytes())
                    .map_err(Refusal::Fault)
            }
            _ => Err(Refusal::Unsupported {
                text: String::from("vector destination"),
            }),
        }
    }

    /// The state effect shared by `movaps`, `movups`, `movdqa`, and `movdqu`.
    fn vector_move(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let value = self.read_vector_operand(instruction, 1)?;
        self.write_vector_operand(instruction, 0, value)?;
        Ok(Outcome::Continued)
    }

    /// `movd` and `movq`: four or eight bytes in or out of a vector register.
    ///
    /// Two things make these different from [`Self::vector_move`], and both
    /// matter. They move *part* of a register, so the width is the
    /// instruction's own rather than the whole hundred and twenty-eight bits.
    /// And when the destination is a vector register the architecture **clears
    /// everything above what was written** — which is what makes `movq` a way
    /// of zeroing the top half, and what a naive read-modify-write would get
    /// wrong in the direction that leaves stale bytes behind.
    ///
    /// Either end can be a vector register, a general register or memory, and
    /// the pairing decides how each end is read and written.
    fn narrow_vector_move(
        &mut self,
        instruction: &Instruction,
        width: usize,
    ) -> Result<Outcome, Refusal> {
        let value = if Self::is_vector_operand(instruction, 1) {
            // The low bytes of the vector, and nothing above them.
            let whole = self.read_vector_operand(instruction, 1)?;
            u128_low(whole, width)
        } else {
            u128::from(self.read_operand(instruction, 1, width)?)
        };

        if Self::is_vector_operand(instruction, 0) {
            // Zero-extended into the whole register: `value` already carries
            // nothing above `width`, so writing it is the clearing.
            self.write_vector_operand(instruction, 0, value)?;
        } else {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the value was masked to `width`, which is at most eight bytes"
            )]
            let narrow = value as u64;
            self.write_operand(instruction, 0, width, narrow)?;
        }
        Ok(Outcome::Continued)
    }

    /// Whether an operand names a vector register, as against a general one or
    /// a location in memory.
    fn is_vector_operand(instruction: &Instruction, operand: u32) -> bool {
        instruction.op_kind(operand) == OpKind::Register
            && instruction.op_register(operand).is_xmm()
    }

    /// The state effect shared by `pxor` and `xorps`.
    fn vector_xor(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let left = self.read_vector_operand(instruction, 0)?;
        let right = self.read_vector_operand(instruction, 1)?;
        self.write_vector_operand(instruction, 0, left ^ right)?;
        Ok(Outcome::Continued)
    }

    /// `movzx` and `movsx`: the source is narrower than the destination, and
    /// which one they are is the whole difference between them.
    fn extend(&mut self, instruction: &Instruction, signed: bool) -> Result<Outcome, Refusal> {
        let destination = Self::operand_size(instruction, 0);
        let source = match instruction.op_kind(1) {
            OpKind::Register => instruction.op_register(1).size(),
            _ => instruction.memory_size().size(),
        };
        let value = self.read_operand(instruction, 1, source)?;
        let widened = if signed {
            sign_extend(value, source)
        } else {
            truncate(value, source)
        };
        self.write_operand(instruction, 0, destination, widened)?;
        Ok(Outcome::Continued)
    }

    /// `lea` computes an address and does not read it. That is its point, and
    /// the reason it is a compiler's favourite way of doing arithmetic.
    fn lea(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let address = self.effective_address(instruction, 1);
        let size = Self::operand_size(instruction, 0);
        self.write_operand(instruction, 0, size, address)?;
        Ok(Outcome::Continued)
    }

    fn xchg(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let first = self.read_operand(instruction, 0, size)?;
        let second = self.read_operand(instruction, 1, size)?;
        self.write_operand(instruction, 0, size, second)?;
        self.write_operand(instruction, 1, size, first)?;
        Ok(Outcome::Continued)
    }

    fn push_operand(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = match instruction.op_kind(0) {
            OpKind::Register => instruction.op_register(0).size(),
            OpKind::Memory => instruction.memory_size().size(),
            _ => self.word() as usize,
        };
        let value = self.read_operand(instruction, 0, size)?;
        // An immediate or a narrow register is widened to the stack's word:
        // the stack pointer moves by a word whatever was pushed.
        self.push(sign_extend(value, size))?;
        Ok(Outcome::Continued)
    }

    fn pop_operand(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let value = self.pop()?;
        let size = Self::operand_size(instruction, 0).max(self.word() as usize);
        self.write_operand(instruction, 0, size, value)?;
        Ok(Outcome::Continued)
    }

    // ----- arithmetic and logic ---------------------------------------------

    fn arithmetic(
        &mut self,
        instruction: &Instruction,
        kind: Arithmetic,
    ) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let left = self.read_operand(instruction, 0, size)?;
        let right = self.read_operand(instruction, 1, size)?;
        let carry = u64::from(self.registers.flag(Flag::Carry));
        let bits = size * 8;
        let result = match kind {
            Arithmetic::Add => left.wrapping_add(right),
            Arithmetic::AddCarry => left.wrapping_add(right).wrapping_add(carry),
            Arithmetic::Sub | Arithmetic::Compare => left.wrapping_sub(right),
            Arithmetic::SubBorrow => left.wrapping_sub(right).wrapping_sub(carry),
            Arithmetic::And | Arithmetic::Test => left & right,
            Arithmetic::Or => left | right,
            Arithmetic::Xor => left ^ right,
        };
        let result = truncate(result, size);
        match kind {
            Arithmetic::Add | Arithmetic::AddCarry => {
                let extra = if kind == Arithmetic::AddCarry {
                    carry
                } else {
                    0
                };
                self.set_add_flags(left, right, extra, result, bits);
            }
            Arithmetic::Sub | Arithmetic::Compare | Arithmetic::SubBorrow => {
                let extra = if kind == Arithmetic::SubBorrow {
                    carry
                } else {
                    0
                };
                self.set_sub_flags(left, right, extra, result, bits);
            }
            _ => self.set_logic_flags(result, bits),
        }
        if !matches!(kind, Arithmetic::Compare | Arithmetic::Test) {
            self.write_operand(instruction, 0, size, result)?;
        }
        Ok(Outcome::Continued)
    }

    /// `inc` and `dec`, which are an `add` and a `sub` of one that leave the
    /// carry flag exactly as they found it. Code relies on that.
    fn step_by_one(&mut self, instruction: &Instruction, up: bool) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let bits = size * 8;
        let value = self.read_operand(instruction, 0, size)?;
        let carry = self.registers.flag(Flag::Carry);
        let result = truncate(
            if up {
                value.wrapping_add(1)
            } else {
                value.wrapping_sub(1)
            },
            size,
        );
        if up {
            self.set_add_flags(value, 1, 0, result, bits);
        } else {
            self.set_sub_flags(value, 1, 0, result, bits);
        }
        self.registers.set_flag(Flag::Carry, carry);
        self.write_operand(instruction, 0, size, result)?;
        Ok(Outcome::Continued)
    }

    fn neg(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let bits = size * 8;
        let value = self.read_operand(instruction, 0, size)?;
        let result = truncate(0_u64.wrapping_sub(value), size);
        self.set_sub_flags(0, value, 0, result, bits);
        // The one place the carry is not what a subtraction from zero gives:
        // it says whether there was anything to negate.
        self.registers.set_flag(Flag::Carry, value != 0);
        self.write_operand(instruction, 0, size, result)?;
        Ok(Outcome::Continued)
    }

    /// `not` touches no flag at all.
    fn not(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let value = self.read_operand(instruction, 0, size)?;
        self.write_operand(instruction, 0, size, truncate(!value, size))?;
        Ok(Outcome::Continued)
    }

    /// `imul` in its three forms: one operand widens into `rdx:rax`, two and
    /// three operands keep only the low half.
    fn imul(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let count = instruction.op_count();
        if count == 1 {
            let size = Self::operand_size(instruction, 0);
            let bits = size * 8;
            let left = sign_extend(self.accumulator(size), size) as i128;
            let right = sign_extend(self.read_operand(instruction, 0, size)?, size) as i128;
            let product = left * right;
            let low = truncate(product as u64, size);
            let fits = product == i128::from(sign_extend(low, size) as i64);
            self.write_accumulator_pair(size, product as u128);
            self.registers.set_flag(Flag::Carry, !fits);
            self.registers.set_flag(Flag::Overflow, !fits);
            let _ = bits;
            return Ok(Outcome::Continued);
        }
        let size = Self::operand_size(instruction, 0);
        let left = sign_extend(self.read_operand(instruction, count - 2, size)?, size) as i128;
        let right = if count == 3 {
            sign_extend(self.read_operand(instruction, 2, size)?, size) as i128
        } else {
            sign_extend(self.read_operand(instruction, 0, size)?, size) as i128
        };
        let product = left * right;
        let low = truncate(product as u64, size);
        let fits = product == i128::from(sign_extend(low, size) as i64);
        self.registers.set_flag(Flag::Carry, !fits);
        self.registers.set_flag(Flag::Overflow, !fits);
        self.write_operand(instruction, 0, size, low)?;
        Ok(Outcome::Continued)
    }

    fn mul(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let left = u128::from(self.accumulator(size));
        let right = u128::from(self.read_operand(instruction, 0, size)?);
        let product = left * right;
        let high = product >> (size * 8);
        self.write_accumulator_pair(size, product);
        self.registers.set_flag(Flag::Carry, high != 0);
        self.registers.set_flag(Flag::Overflow, high != 0);
        Ok(Outcome::Continued)
    }

    /// `div` and `idiv`, and the two ways they have of not having an answer:
    /// a divisor of zero, and a quotient too big for the register it goes in.
    /// Both are the same fault on a real processor, and both stop the run.
    fn divide(&mut self, instruction: &Instruction, signed: bool) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let divisor = self.read_operand(instruction, 0, size)?;
        if divisor == 0 {
            return Err(Refusal::DivideError);
        }
        let dividend = self.accumulator_pair(size);
        let (quotient, remainder) = if signed {
            let dividend = dividend as i128;
            let divisor = i128::from(sign_extend(divisor, size) as i64);
            let quotient = dividend.checked_div(divisor).ok_or(Refusal::DivideError)?;
            let limit = 1_i128 << (size * 8 - 1);
            if quotient >= limit || quotient < -limit {
                return Err(Refusal::DivideError);
            }
            (quotient as u128, (dividend % divisor) as u128)
        } else {
            let divisor = u128::from(divisor);
            let quotient = dividend / divisor;
            if quotient >> (size * 8) != 0 {
                return Err(Refusal::DivideError);
            }
            (quotient, dividend % divisor)
        };
        self.write_quotient(size, quotient as u64, remainder as u64);
        Ok(Outcome::Continued)
    }

    /// The accumulator at the width an instruction works in: `al`, `ax`,
    /// `eax` or `rax`.
    fn accumulator(&self, size: usize) -> u64 {
        self.registers.get(accumulator_register(size))
    }

    /// `dx:ax`, `edx:eax` or `rdx:rax` as one value, and `ax` alone for the
    /// byte-wide form, which is what the architecture uses there.
    fn accumulator_pair(&self, size: usize) -> u128 {
        if size == 1 {
            return u128::from(self.registers.get(Register::AX));
        }
        let low = u128::from(self.accumulator(size));
        let high = u128::from(self.registers.get(data_register(size)));
        (high << (size * 8)) | low
    }

    /// Writes a double-width product back where the architecture puts it.
    fn write_accumulator_pair(&mut self, size: usize, value: u128) {
        if size == 1 {
            self.registers.set(Register::AX, truncate(value as u64, 2));
            return;
        }
        self.registers
            .set(accumulator_register(size), truncate(value as u64, size));
        let high = (value >> (size * 8)) as u64;
        self.registers
            .set(data_register(size), truncate(high, size));
    }

    /// Writes a quotient and its remainder where a division puts them.
    fn write_quotient(&mut self, size: usize, quotient: u64, remainder: u64) {
        if size == 1 {
            self.registers.set(Register::AL, truncate(quotient, 1));
            self.registers.set(Register::AH, truncate(remainder, 1));
            return;
        }
        self.registers
            .set(accumulator_register(size), truncate(quotient, size));
        self.registers
            .set(data_register(size), truncate(remainder, size));
    }

    // ----- shifts and bits --------------------------------------------------

    fn shift(&mut self, instruction: &Instruction, kind: Shift) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let bits = (size * 8) as u32;
        let value = self.read_operand(instruction, 0, size)?;
        // The count is taken modulo the register width, and a sixty-four bit
        // shift masks to six bits rather than five. A count of zero changes
        // nothing, flags included — the case that is easy to get wrong.
        let mask = if bits == 64 { 63 } else { 31 };
        let raw = self.read_operand(instruction, 1, 1)? & mask;
        let count = match kind {
            Shift::RotateLeft | Shift::RotateRight => raw % u64::from(bits),
            _ => raw,
        };
        if raw == 0 {
            return Ok(Outcome::Continued);
        }
        let (result, carry) = match kind {
            Shift::Left => {
                let shifted = value.wrapping_shl(count as u32);
                let out = count <= u64::from(bits) && (value >> (u64::from(bits) - count)) & 1 == 1;
                (truncate(shifted, size), out)
            }
            Shift::Right => {
                let value = truncate(value, size);
                let out = count <= u64::from(bits) && (value >> (count - 1)) & 1 == 1;
                (value.wrapping_shr(count as u32), out)
            }
            Shift::Arithmetic => {
                let signed = sign_extend(value, size);
                let shift = count.min(u64::from(bits) - 1) as u32;
                let out = (signed >> shift.min(63)) & 1 == 1;
                let result = ((signed as i64) >> shift) as u64;
                (truncate(result, size), out)
            }
            Shift::RotateLeft | Shift::RotateRight => {
                if count == 0 {
                    // A rotate by a whole number of turns still touches the
                    // carry, which is why it is not the same as a count of zero.
                    let value = truncate(value, size);
                    let carry = if kind == Shift::RotateLeft {
                        value & 1 == 1
                    } else {
                        (value >> (bits - 1)) & 1 == 1
                    };
                    self.registers.set_flag(Flag::Carry, carry);
                    self.write_operand(instruction, 0, size, value)?;
                    return Ok(Outcome::Continued);
                }
                let value = truncate(value, size);
                let count = count as u32;
                let rotated = if kind == Shift::RotateLeft {
                    (value << count) | (value >> (bits - count))
                } else {
                    (value >> count) | (value << (bits - count))
                };
                let rotated = truncate(rotated, size);
                let carry = if kind == Shift::RotateLeft {
                    rotated & 1 == 1
                } else {
                    (rotated >> (bits - 1)) & 1 == 1
                };
                (rotated, carry)
            }
        };
        self.registers.set_flag(Flag::Carry, carry);
        if matches!(kind, Shift::RotateLeft | Shift::RotateRight) {
            self.write_operand(instruction, 0, size, result)?;
            return Ok(Outcome::Continued);
        }
        self.set_result_flags(result, size * 8);
        // A one-place shift has a defined overflow flag; wider ones do not,
        // and leaving the flag alone is closer to the truth than a guess.
        if count == 1 {
            let overflow = match kind {
                Shift::Left => ((result >> (bits - 1)) & 1 == 1) != carry,
                Shift::Right => (value >> (bits - 1)) & 1 == 1,
                _ => false,
            };
            self.registers.set_flag(Flag::Overflow, overflow);
        }
        self.write_operand(instruction, 0, size, result)?;
        Ok(Outcome::Continued)
    }

    /// `bt`, `bts`, `btr`, `btc`: the carry flag receives the bit, and three
    /// of the four then change it.
    fn bit(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let bits = (size * 8) as u64;
        let value = self.read_operand(instruction, 0, size)?;
        let position = self.read_operand(instruction, 1, size)? % bits;
        let bit = (value >> position) & 1 == 1;
        self.registers.set_flag(Flag::Carry, bit);
        let changed = match instruction.mnemonic() {
            Mnemonic::Bts => value | (1 << position),
            Mnemonic::Btr => value & !(1 << position),
            Mnemonic::Btc => value ^ (1 << position),
            _ => return Ok(Outcome::Continued),
        };
        self.write_operand(instruction, 0, size, truncate(changed, size))?;
        Ok(Outcome::Continued)
    }

    /// `bsf`, `bsr`, `tzcnt`, `lzcnt`: finding a set bit, and what each of
    /// them does when there is none.
    fn bit_scan(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let bits = (size * 8) as u32;
        let value = truncate(self.read_operand(instruction, 1, size)?, size);
        let mnemonic = instruction.mnemonic();
        if value == 0 {
            match mnemonic {
                // The count instructions answer the width; the scans leave the
                // destination as it was and only say the source was zero.
                Mnemonic::Tzcnt | Mnemonic::Lzcnt => {
                    self.registers.set_flag(Flag::Carry, true);
                    self.registers.set_flag(Flag::Zero, false);
                    self.write_operand(instruction, 0, size, u64::from(bits))?;
                }
                _ => self.registers.set_flag(Flag::Zero, true),
            }
            return Ok(Outcome::Continued);
        }
        let position = match mnemonic {
            Mnemonic::Bsf | Mnemonic::Tzcnt => value.trailing_zeros(),
            _ => {
                let leading = value.leading_zeros() - (64 - bits);
                if mnemonic == Mnemonic::Lzcnt {
                    leading
                } else {
                    bits - 1 - leading
                }
            }
        };
        self.registers.set_flag(Flag::Zero, false);
        if matches!(mnemonic, Mnemonic::Tzcnt | Mnemonic::Lzcnt) {
            self.registers.set_flag(Flag::Carry, false);
        }
        self.write_operand(instruction, 0, size, u64::from(position))?;
        Ok(Outcome::Continued)
    }

    fn popcnt(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let value = truncate(self.read_operand(instruction, 1, size)?, size);
        self.set_logic_flags(0, size * 8);
        self.registers.set_flag(Flag::Zero, value == 0);
        self.write_operand(instruction, 0, size, u64::from(value.count_ones()))?;
        Ok(Outcome::Continued)
    }

    fn bswap(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let value = truncate(self.read_operand(instruction, 0, size)?, size);
        let swapped = value.swap_bytes() >> ((8 - size.clamp(1, 8)) * 8);
        self.write_operand(instruction, 0, size, swapped)?;
        Ok(Outcome::Continued)
    }

    /// `cbw`, `cwde`, `cdqe`: widening the accumulator into itself.
    fn sign_extend_accumulator(&mut self, instruction: &Instruction) -> Outcome {
        let (from, to) = match instruction.mnemonic() {
            Mnemonic::Cbw => (1, 2),
            Mnemonic::Cwde => (2, 4),
            _ => (4, 8),
        };
        let value = sign_extend(self.registers.get(accumulator_register(from)), from);
        self.registers
            .set(accumulator_register(to), truncate(value, to));
        Outcome::Continued
    }

    /// `cwd`, `cdq`, `cqo`: filling `rdx` with the sign of `rax`, which is
    /// what a signed division needs before it can start.
    fn sign_extend_into_dx(&mut self, instruction: &Instruction) -> Outcome {
        let size = match instruction.mnemonic() {
            Mnemonic::Cwd => 2,
            Mnemonic::Cdq => 4,
            _ => 8,
        };
        let value = self.registers.get(accumulator_register(size));
        let negative = (value >> (size * 8 - 1)) & 1 == 1;
        let fill = if negative { u64::MAX } else { 0 };
        self.registers
            .set(data_register(size), truncate(fill, size));
        Outcome::Continued
    }

    // ----- conditions and flow ----------------------------------------------

    /// Whether the condition a `jcc`, `setcc` or `cmovcc` names holds.
    fn condition_holds(&self, instruction: &Instruction) -> Option<bool> {
        let carry = self.registers.flag(Flag::Carry);
        let zero = self.registers.flag(Flag::Zero);
        let sign = self.registers.flag(Flag::Sign);
        let overflow = self.registers.flag(Flag::Overflow);
        let parity = self.registers.flag(Flag::Parity);
        Some(match instruction.condition_code() {
            ConditionCode::o => overflow,
            ConditionCode::no => !overflow,
            ConditionCode::b => carry,
            ConditionCode::ae => !carry,
            ConditionCode::e => zero,
            ConditionCode::ne => !zero,
            ConditionCode::be => carry || zero,
            ConditionCode::a => !carry && !zero,
            ConditionCode::s => sign,
            ConditionCode::ns => !sign,
            ConditionCode::p => parity,
            ConditionCode::np => !parity,
            ConditionCode::l => sign != overflow,
            ConditionCode::ge => sign == overflow,
            ConditionCode::le => zero || (sign != overflow),
            ConditionCode::g => !zero && (sign == overflow),
            ConditionCode::None => return None,
        })
    }

    fn cmovcc(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let holds = self.condition_holds(instruction).unwrap_or(false);
        // The read happens either way: a `cmov` from unmapped memory faults
        // even when the condition does not hold, which is why compilers do not
        // use it to guard a pointer.
        let value = self.read_operand(instruction, 1, size)?;
        if holds {
            self.write_operand(instruction, 0, size, value)?;
        } else if size == 4 {
            // The other rule that surprises people: a thirty-two bit `cmov`
            // clears the top half of the register whether it moved or not.
            let kept = self.read_operand(instruction, 0, size)?;
            self.write_operand(instruction, 0, size, kept)?;
        }
        Ok(Outcome::Continued)
    }

    /// `cpuid` — what this processor is, answered by this processor.
    ///
    /// Not an invention about the file: it is a question about the machine the
    /// code is running on, and the machine it is running on is the one this
    /// module builds. The honest answer is therefore what this interpreter
    /// really carries out — a 64-bit x86 with SSE2 and nothing above it.
    ///
    /// Answering *more* would be the dangerous direction. glibc asks `cpuid`
    /// precisely to choose between an SSE2 `memcpy`, an AVX one and an AVX-512
    /// one; claiming AVX would send it down a path of instructions this
    /// interpreter cannot execute, and the run would stop a hundred
    /// instructions later on something far harder to understand than a
    /// `cpuid`.
    fn cpuid(&mut self) -> Result<Outcome, Refusal> {
        use iced_x86::Register as R;
        let leaf = self.registers.get(R::EAX);
        let (eax, ebx, ecx, edx) = match leaf {
            // Highest leaf supported, and the vendor string in EBX:EDX:ECX.
            // "DesdecVirtualCPU" is not a vendor anyone tests for, which is
            // the point: nothing should take a path meant for real silicon.
            0 => (1, 0x6365_7344, 0x5550_4356, 0x6c61_7574),
            // Feature bits. EDX bit 26 is SSE2, bit 25 SSE, bit 24 FXSR,
            // bit 23 MMX, bit 15 CMOV, bit 8 CMPXCHG8B, bit 4 TSC, bit 0 FPU.
            // ECX bit 13 is CMPXCHG16B, which 64-bit code assumes. Nothing
            // above SSE2 is claimed.
            1 => (0x0000_0f00, 0, 0x0000_2000, 0x0788_a131),
            // Extended leaves: the highest one, and long mode in leaf
            // 0x80000001 EDX bit 29.
            0x8000_0000 => (0x8000_0001, 0, 0, 0),
            0x8000_0001 => (0, 0, 0, 0x2000_0000),
            // Anything else is answered with zeroes, which is what a
            // processor does for a leaf it does not implement.
            _ => (0, 0, 0, 0),
        };
        self.registers.set(R::RAX, eax);
        self.registers.set(R::RBX, ebx);
        self.registers.set(R::RCX, ecx);
        self.registers.set(R::RDX, edx);
        Ok(Outcome::Continued)
    }

    /// `rdtsc` and `rdtscp` — the cycle counter.
    ///
    /// A count of instructions retired rather than of cycles, which is the
    /// only clock this machine has and is the one property a reader can check
    /// against the trace. It advances, it never goes backwards, and it is the
    /// same on every run of the same program — which a real counter is not,
    /// and which is what makes a decompiler's run repeatable.
    fn rdtsc(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        use iced_x86::Register as R;
        let ticks = self.retired;
        self.registers.set(R::RAX, ticks & 0xFFFF_FFFF);
        self.registers.set(R::RDX, ticks >> 32);
        // `rdtscp` also reports which processor answered. There is one.
        if instruction.mnemonic() == Mnemonic::Rdtscp {
            self.registers.set(R::RCX, 0);
        }
        Ok(Outcome::Continued)
    }

    /// `xgetbv` — which extended processor state is enabled.
    ///
    /// None of it, consistently with [`Self::cpuid`] claiming nothing above
    /// SSE2: bit 0 is the x87 state and bit 1 the SSE state, and the AVX bit
    /// is deliberately clear.
    fn xgetbv(&mut self) -> Result<Outcome, Refusal> {
        use iced_x86::Register as R;
        self.registers.set(R::RAX, 0b11);
        self.registers.set(R::RDX, 0);
        Ok(Outcome::Continued)
    }

    fn setcc(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let holds = self.condition_holds(instruction).unwrap_or(false);
        self.write_operand(instruction, 0, 1, u64::from(holds))?;
        Ok(Outcome::Continued)
    }

    fn jump(&mut self, instruction: &Instruction, text: &str) -> Result<Outcome, Refusal> {
        let target = self.branch_target(instruction, text)?;
        self.registers.instruction_pointer = target;
        Ok(Outcome::Continued)
    }

    fn jcc(&mut self, instruction: &Instruction, text: &str) -> Result<Outcome, Refusal> {
        if self.condition_holds(instruction) == Some(true) {
            let target = self.branch_target(instruction, text)?;
            self.registers.instruction_pointer = target;
        }
        Ok(Outcome::Continued)
    }

    fn call(&mut self, instruction: &Instruction, text: &str) -> Result<Outcome, Refusal> {
        let returns_to = instruction.next_ip();
        let target = self.branch_target(instruction, text)?;
        self.push(returns_to)?;
        self.registers.instruction_pointer = target;
        Ok(Outcome::Called { returns_to })
    }

    /// Where a branch goes: a fixed address, or one read from a register or
    /// from memory — which the emulator has, unlike a static reading.
    fn branch_target(&mut self, instruction: &Instruction, text: &str) -> Result<u64, Refusal> {
        match instruction.op_kind(0) {
            OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
                Ok(instruction.near_branch_target())
            }
            OpKind::Register => Ok(self.registers.get(instruction.op_register(0))),
            OpKind::Memory => {
                let size = instruction.memory_size().size().clamp(1, 8);
                let address = self.effective_address(instruction, 0);
                self.branched_through = Some(address);
                self.load(address, size)
            }
            // A far branch changes the code segment, and there is no segment
            // table here to change it to.
            _ => Err(Refusal::Unsupported { text: text.into() }),
        }
    }

    fn ret(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let address = self.pop()?;
        // `ret $16` drops arguments the caller pushed, on top of the address.
        if instruction.op_count() > 0 {
            let dropped = self.read_operand(instruction, 0, 2)?;
            let top = self.registers.stack_pointer().wrapping_add(dropped);
            self.registers.set_stack_pointer(top);
        }
        self.registers.instruction_pointer = address;
        Ok(Outcome::Returned)
    }

    /// `leave` is `mov %rbp,%rsp` then `pop %rbp`, and nothing else.
    fn leave(&mut self) -> Result<Outcome, Refusal> {
        let frame = if self.bitness == 64 {
            Register::RBP
        } else {
            Register::EBP
        };
        let base = self.registers.get(frame);
        self.registers.set_stack_pointer(base);
        let saved = self.pop()?;
        self.registers.set(frame, saved);
        Ok(Outcome::Continued)
    }

    fn cmpxchg(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let bits = size * 8;
        let destination = self.read_operand(instruction, 0, size)?;
        let expected = self.accumulator(size);
        let difference = truncate(expected.wrapping_sub(destination), size);
        self.set_sub_flags(expected, destination, 0, difference, bits);
        if difference == 0 {
            let source = self.read_operand(instruction, 1, size)?;
            self.write_operand(instruction, 0, size, source)?;
        } else {
            self.registers
                .set(accumulator_register(size), truncate(destination, size));
        }
        Ok(Outcome::Continued)
    }

    fn xadd(&mut self, instruction: &Instruction) -> Result<Outcome, Refusal> {
        let size = Self::operand_size(instruction, 0);
        let bits = size * 8;
        let destination = self.read_operand(instruction, 0, size)?;
        let source = self.read_operand(instruction, 1, size)?;
        let sum = truncate(destination.wrapping_add(source), size);
        self.set_add_flags(destination, source, 0, sum, bits);
        self.write_operand(instruction, 1, size, destination)?;
        self.write_operand(instruction, 0, size, sum)?;
        Ok(Outcome::Continued)
    }

    // ----- string instructions ----------------------------------------------

    /// `movs`, `stos`, `lods`, `scas` and `cmps`, with their repeat prefixes.
    ///
    /// One iteration is carried out per call, and the instruction pointer is
    /// put back where it was while the count still has something in it: the
    /// reader stepping through a `rep movsb` sees it move one byte at a time,
    /// which is what a debugger's single step does on a real processor too.
    fn string_operation(
        &mut self,
        instruction: &Instruction,
        operation: StringOperation,
        text: &str,
    ) -> Result<Outcome, Refusal> {
        let _ = text;
        let size = string_width(instruction.code());
        let repeat = instruction.has_rep_prefix()
            || instruction.has_repe_prefix()
            || instruction.has_repne_prefix();
        let counter = if self.bitness == 64 {
            Register::RCX
        } else {
            Register::ECX
        };
        if repeat && self.registers.get(counter) == 0 {
            return Ok(Outcome::Continued);
        }
        let step = if self.registers.flag(Flag::Direction) {
            0_u64.wrapping_sub(size as u64)
        } else {
            size as u64
        };
        let source_register = if self.bitness == 64 {
            Register::RSI
        } else {
            Register::ESI
        };
        let destination_register = if self.bitness == 64 {
            Register::RDI
        } else {
            Register::EDI
        };
        let source = self.registers.get(source_register);
        let destination = self.registers.get(destination_register);
        let mut compared: Option<(u64, u64)> = None;
        match operation {
            StringOperation::Move => {
                let value = self.load(source, size)?;
                self.store(destination, size, value)?;
                self.registers
                    .set(source_register, source.wrapping_add(step));
                self.registers
                    .set(destination_register, destination.wrapping_add(step));
            }
            StringOperation::Store => {
                let value = self.accumulator(size);
                self.store(destination, size, value)?;
                self.registers
                    .set(destination_register, destination.wrapping_add(step));
            }
            StringOperation::Load => {
                let value = self.load(source, size)?;
                self.registers
                    .set(accumulator_register(size), truncate(value, size));
                self.registers
                    .set(source_register, source.wrapping_add(step));
            }
            StringOperation::Scan => {
                let value = self.load(destination, size)?;
                compared = Some((self.accumulator(size), value));
                self.registers
                    .set(destination_register, destination.wrapping_add(step));
            }
            StringOperation::Compare => {
                let left = self.load(source, size)?;
                let right = self.load(destination, size)?;
                compared = Some((left, right));
                self.registers
                    .set(source_register, source.wrapping_add(step));
                self.registers
                    .set(destination_register, destination.wrapping_add(step));
            }
        }
        if let Some((left, right)) = compared {
            let difference = truncate(left.wrapping_sub(right), size);
            self.set_sub_flags(left, right, 0, difference, size * 8);
        }
        if !repeat {
            return Ok(Outcome::Continued);
        }
        let remaining = self.registers.get(counter).wrapping_sub(1);
        self.registers.set(counter, remaining);
        // `repe` and `repne` stop early on the flag as well as on the count —
        // but only where the flag means anything. The same `f3` byte is `rep`
        // in front of a `movs`, which settles no flag, and `repe` in front of a
        // `cmps`, which does; reading it as `repe` everywhere would end every
        // `rep movs` after one iteration.
        let condition_ends = match operation {
            StringOperation::Scan | StringOperation::Compare => {
                if instruction.has_repne_prefix() {
                    self.registers.flag(Flag::Zero)
                } else {
                    !self.registers.flag(Flag::Zero)
                }
            }
            _ => false,
        };
        if remaining != 0 && !condition_ends {
            self.registers.instruction_pointer = instruction.ip();
        }
        Ok(Outcome::Continued)
    }

    // ----- flags ------------------------------------------------------------

    /// The three flags every result settles the same way, plus parity.
    fn set_result_flags(&mut self, result: u64, bits: usize) {
        self.registers.set_flag(Flag::Zero, result == 0);
        self.registers
            .set_flag(Flag::Sign, (result >> (bits - 1)) & 1 == 1);
        self.registers
            .set_flag(Flag::Parity, (result & 0xff).count_ones() % 2 == 0);
    }

    /// A logical result: carry and overflow are cleared, not left alone.
    fn set_logic_flags(&mut self, result: u64, bits: usize) {
        self.set_result_flags(result, bits);
        self.registers.set_flag(Flag::Carry, false);
        self.registers.set_flag(Flag::Overflow, false);
        self.registers.set_flag(Flag::Adjust, false);
    }

    fn set_add_flags(&mut self, left: u64, right: u64, extra: u64, result: u64, bits: usize) {
        self.set_result_flags(result, bits);
        let top = 1_u64 << (bits - 1);
        let carry = if bits == 64 {
            let (sum, first) = left.overflowing_add(right);
            let (_, second) = sum.overflowing_add(extra);
            first || second
        } else {
            let mask = (1_u64 << bits) - 1;
            ((left & mask) + (right & mask) + extra) > mask
        };
        self.registers.set_flag(Flag::Carry, carry);
        // Signed overflow: two operands of the same sign gave a result of the
        // other. Two operands of different signs can never overflow.
        let overflow = ((left ^ result) & (right ^ result) & top) != 0;
        self.registers.set_flag(Flag::Overflow, overflow);
        self.registers
            .set_flag(Flag::Adjust, ((left ^ right ^ result) & 0x10) != 0);
    }

    fn set_sub_flags(&mut self, left: u64, right: u64, extra: u64, result: u64, bits: usize) {
        self.set_result_flags(result, bits);
        let top = 1_u64 << (bits - 1);
        let mask = if bits == 64 {
            u64::MAX
        } else {
            (1_u64 << bits) - 1
        };
        let borrow = u128::from(left & mask) < u128::from(right & mask) + u128::from(extra);
        self.registers.set_flag(Flag::Carry, borrow);
        let overflow = ((left ^ right) & (left ^ result) & top) != 0;
        self.registers.set_flag(Flag::Overflow, overflow);
        self.registers
            .set_flag(Flag::Adjust, ((left ^ right ^ result) & 0x10) != 0);
    }
}

/// Which shape of arithmetic an instruction is, for the flags it settles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Arithmetic {
    Add,
    AddCarry,
    Sub,
    SubBorrow,
    Compare,
    And,
    Or,
    Xor,
    Test,
}

/// Which shape of shift or rotate an instruction is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Shift {
    Left,
    Right,
    Arithmetic,
    RotateLeft,
    RotateRight,
}

/// The same for a vector-wide value: the low `size` bytes, nothing above.
const fn u128_low(value: u128, size: usize) -> u128 {
    match size {
        4 => value & 0xffff_ffff,
        8 => value & 0xffff_ffff_ffff_ffff,
        _ => value,
    }
}

/// Keeps the low `size` bytes of a value and clears the rest.
const fn truncate(value: u64, size: usize) -> u64 {
    match size {
        1 => value & 0xff,
        2 => value & 0xffff,
        4 => value & 0xffff_ffff,
        _ => value,
    }
}

/// Widens a value of `size` bytes to sixty-four, repeating its top bit.
const fn sign_extend(value: u64, size: usize) -> u64 {
    match size {
        1 => value as u8 as i8 as i64 as u64,
        2 => value as u16 as i16 as i64 as u64,
        4 => value as u32 as i32 as i64 as u64,
        _ => value,
    }
}

/// `al`, `ax`, `eax` or `rax`, by width.
const fn accumulator_register(size: usize) -> Register {
    match size {
        1 => Register::AL,
        2 => Register::AX,
        4 => Register::EAX,
        _ => Register::RAX,
    }
}

/// `dl`, `dx`, `edx` or `rdx`, by width.
const fn data_register(size: usize) -> Register {
    match size {
        1 => Register::DL,
        2 => Register::DX,
        4 => Register::EDX,
        _ => Register::RDX,
    }
}

/// Which of the five string instructions an opcode is.
///
/// The decoder names them by width — `movsb`, `movsw`, `movsd`, `movsq` — and
/// one of those names, `movsd`, is also an SSE instruction that moves a double.
/// Matching the opcode rather than the mnemonic is what keeps the two apart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StringOperation {
    Move,
    Store,
    Load,
    Scan,
    Compare,
}

/// The string instruction an opcode is, if it is one at all.
const fn string_operation_of(code: Code) -> Option<StringOperation> {
    Some(match code {
        Code::Movsb_m8_m8 | Code::Movsw_m16_m16 | Code::Movsd_m32_m32 | Code::Movsq_m64_m64 => {
            StringOperation::Move
        }
        Code::Stosb_m8_AL | Code::Stosw_m16_AX | Code::Stosd_m32_EAX | Code::Stosq_m64_RAX => {
            StringOperation::Store
        }
        Code::Lodsb_AL_m8 | Code::Lodsw_AX_m16 | Code::Lodsd_EAX_m32 | Code::Lodsq_RAX_m64 => {
            StringOperation::Load
        }
        Code::Scasb_AL_m8 | Code::Scasw_AX_m16 | Code::Scasd_EAX_m32 | Code::Scasq_RAX_m64 => {
            StringOperation::Scan
        }
        Code::Cmpsb_m8_m8 | Code::Cmpsw_m16_m16 | Code::Cmpsd_m32_m32 | Code::Cmpsq_m64_m64 => {
            StringOperation::Compare
        }
        _ => return None,
    })
}

/// How many bytes one iteration of a string instruction moves.
const fn string_width(code: Code) -> usize {
    match code {
        Code::Movsb_m8_m8
        | Code::Stosb_m8_AL
        | Code::Lodsb_AL_m8
        | Code::Scasb_AL_m8
        | Code::Cmpsb_m8_m8 => 1,
        Code::Movsw_m16_m16
        | Code::Stosw_m16_AX
        | Code::Lodsw_AX_m16
        | Code::Scasw_AX_m16
        | Code::Cmpsw_m16_m16 => 2,
        Code::Movsq_m64_m64
        | Code::Stosq_m64_RAX
        | Code::Lodsq_RAX_m64
        | Code::Scasq_RAX_m64
        | Code::Cmpsq_m64_m64 => 8,
        _ => 4,
    }
}
