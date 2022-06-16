#[allow(non_upper_case_globals)]

use crate::bus::Bus;
use crate::{DRAM_BASE, DRAM_END};
use crate::exception::RvException::{self, IllegalInstruction};
use crate::interrupt::RvInterrupt;
use crate::csr::*;


// Riscv Privilege Mode
type Mode = u64;
const User: Mode = 0b00;
const Supervisor: Mode = 0b01;
const Machine: Mode = 0b11;


pub struct Cpu {
    pub regs: [u64; 32],
    pub pc: u64,
    pub bus: Bus,
    pub mode: Mode,
    pub csr: Csr,
}


const RVABI: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", 
    "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5", 
    "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", 
    "s8", "s9", "s10", "s11", "t3", "t4", "t5", "t6",
];
 

impl Cpu {
    pub fn new(code: Vec<u8>) -> Self {
        let mut regs = [0; 32];
        regs[2] = DRAM_END;

        let bus = Bus::new(code);
        let csr = Csr::new();
        let mode = Machine;

        Self {regs, pc: DRAM_BASE, bus, csr, mode}
    }

    pub fn load(&mut self, addr: u64, size: u64) -> Result<u64, RvException> {
        self.bus.load(addr, size)
    }

    pub fn reg(&self, r: &str) -> u64 {
        match RVABI.iter().position(|&x| x == r) {
            Some(i) => self.regs[i],
            None => match r {
                "pc" => self.pc,
                "fp" => self.reg("s0"),
                r if r.starts_with("x") => {
                    if let Ok(i) = r[1..].parse::<usize>() {
                        if i <= 31 { return self.regs[i]; }
                        panic!("Invalid register {}", r);
                    }
                    panic!("Invalid register {}", r);
                }
                "mhartid" => self.csr.load(MHARTID),
                "mstatus" => self.csr.load(MSTATUS),
                "mtvec" => self.csr.load(MTVEC),
                "mepc" => self.csr.load(MEPC),
                "mcause" => self.csr.load(MCAUSE),
                "mtval" => self.csr.load(MTVAL),
                "medeleg" => self.csr.load(MEDELEG),
                "mscratch" => self.csr.load(MSCRATCH),
                "MIP" => self.csr.load(MIP),
                "mcounteren" => self.csr.load(MCOUNTEREN),
                "sstatus" => self.csr.load(SSTATUS),
                "stvec" => self.csr.load(STVEC),
                "sepc" => self.csr.load(SEPC),
                "scause" => self.csr.load(SCAUSE),
                "stval" => self.csr.load(STVAL),
                "sscratch" => self.csr.load(SSCRATCH),
                "SIP" => self.csr.load(SIP),
                "SATP" => self.csr.load(SATP),
                _ => panic!("Invalid register {}", r),
            }
        }
    }

    pub fn store(&mut self, addr: u64, size: u64, value: u64) -> Result<(), RvException> {
        self.bus.store(addr, size, value)
    }

    pub fn dump_pc(&self) {
        println!("{:-^80}", "PC register");
        println!("PC = {:#x}\n", self.pc);
    }

    pub fn dump_registers(&mut self) {
        println!("{:-^80}", "registers");
        let mut output = String::new();
        self.regs[0] = 0;

        for i in (0..32).step_by(4) {
            let i0 = format!("x{}", i);
            let i1 = format!("x{}", i + 1); 
            let i2 = format!("x{}", i + 2);
            let i3 = format!("x{}", i + 3); 
            let line = format!(
                "{:3}({:^4}) = {:<#18x} {:3}({:^4}) = {:<#18x} {:3}({:^4}) = {:<#18x} {:3}({:^4}) = {:<#18x}\n",
                i0, RVABI[i], self.regs[i], 
                i1, RVABI[i + 1], self.regs[i + 1], 
                i2, RVABI[i + 2], self.regs[i + 2], 
                i3, RVABI[i + 3], self.regs[i + 3],
            );
            output = output + &line;
        }

        println!("{}", output);
    }

    pub fn fetch(&mut self) -> Result<u64, RvException> {
        self.bus.load(self.pc, 32)
    }

    pub fn handle_exception(&mut self, e: RvException) {
        // the process to handle exception in S-mode and M-mode is similar,
        // includes following steps:
        // 0. set xPP to current mode.
        // 1. update hart's privilege mode (M or S according to current mode and exception setting).
        // 2. save current pc in epc (sepc in S-mode, mepc in M-mode)
        // 3. set pc to trap vector (stvec in S-mode, mtvec in M-mode)
        // 4. set cause to exception code (scause in S-mode, mcause in M-mode)
        // 5. set trap value properly (stval in S-mode, mtval in M-mode)
        // 6. set xPIE to xIE (SPIE in S-mode, MPIE in M-mode)
        // 7. clear up xIE (SIE in S-mode, MIE in M-mode)
        use RvException::*;
        let pc = self.pc; 
        let mode = self.mode;
        let cause = e.code();
        // if an exception happen in U-mode or S-mode, and the exception is delegated to S-mode.
        // then this exception should be handled in S-mode.
        let trap_in_s_mode = mode <= Supervisor && self.csr.is_medelegated(cause);
        let (STATUS, TVEC, CAUSE, TVAL, EPC, MASK_PIE, pie_i, MASK_IE, ie_i, MASK_PP, pp_i) 
            = if trap_in_s_mode {
                self.mode = Supervisor;
                (SSTATUS, STVEC, SCAUSE, STVAL, SEPC, MASK_SPIE, 5, MASK_SIE, 1, MASK_SPP, 8)
            } else {
                self.mode = Machine;
                (MSTATUS, MTVEC, MCAUSE, MTVAL, MEPC, MASK_MPIE, 7, MASK_MIE, 3, MASK_MPP, 11)
            };
        // 3.1.7 & 4.1.2
        // The BASE field in tvec is a WARL field that can hold any valid virtual or physical address,
        // subject to the following alignment constraints: the address must be 4-byte aligned
        self.pc = self.csr.load(TVEC) & !0b11;
        // 3.1.14 & 4.1.7
        // When a trap is taken into S-mode (or M-mode), sepc (or mepc) is written with the virtual address 
        // of the instruction that was interrupted or that encountered the exception.
        self.csr.store(EPC, pc);
        // 3.1.15 & 4.1.8
        // When a trap is taken into S-mode (or M-mode), scause (or mcause) is written with a code indicating 
        // the event that caused the trap.
        self.csr.store(CAUSE, cause);
        // 3.1.16 & 4.1.9
        // If stval is written with a nonzero value when a breakpoint, address-misaligned, access-fault, or
        // page-fault exception occurs on an instruction fetch, load, or store, then stval will contain the
        // faulting virtual address.
        // If stval is written with a nonzero value when a misaligned load or store causes an access-fault or
        // page-fault exception, then stval will contain the virtual address of the portion of the access that
        // caused the fault
        let addr = match e {
            InstructionAddrMisaligned(addr) 
            | InstructionAccessFault(addr) 
            | InstructionPageFault(addr) => addr,
            LoadAccessMisaligned(addr)
            | LoadAccessFault(addr)
            | LoadPageFault(addr) => addr,
            StoreOrAMOAddrMisaligned(addr)
            | StoreOrAMOAccessFault(addr)
            | StoreOrAMOPageFault(addr) => addr,
            _ => 0,
        };
        self.csr.store(TVAL, addr);
        // 3.1.6 covers both sstatus and mstatus.
        let mut status = self.csr.load(STATUS);
        // get SIE or MIE
        let ie = (status & MASK_IE) >> ie_i;
        // set SPIE = SIE / MPIE = MIE
        status |= ie << pie_i;
        // set SIE = 0 / MIE = 0
        status &= !MASK_IE; 
        // set SPP / MPP = previous mode
        status = (status & !MASK_PP) | (mode << pp_i);
        self.csr.store(STATUS, status);
    }

    pub fn handle_interrupt(&mut self, interrupt: RvInterrupt) {
        // similar to handle exception
        let pc = self.pc; 
        let mode = self.mode;
        let cause = interrupt.code();
        // although cause contains a interrupt bit. Shift the cause make it out.
        let trap_in_s_mode = mode <= Supervisor && self.csr.is_midelegated(cause);
        let (STATUS, TVEC, CAUSE, TVAL, EPC, MASK_PIE, pie_i, MASK_IE, ie_i, MASK_PP, pp_i) 
            = if trap_in_s_mode {
                self.mode = Supervisor;
                (SSTATUS, STVEC, SCAUSE, STVAL, SEPC, MASK_SPIE, 5, MASK_SIE, 1, MASK_SPP, 8)
            } else {
                self.mode = Machine;
                (MSTATUS, MTVEC, MCAUSE, MTVAL, MEPC, MASK_MPIE, 7, MASK_MIE, 3, MASK_MPP, 11)
            };
        // 3.1.7 & 4.1.2
        // When MODE=Direct, all traps into machine mode cause the pc to be set to the address in the BASE field. 
        // When MODE=Vectored, all synchronous exceptions into machine mode cause the pc to be set to the address 
        // in the BASE field, whereas interrupts cause the pc to be set to the address in the BASE field plus four 
        // times the interrupt cause number. 
        let tvec = self.csr.load(TVEC);
        let tvec_mode = tvec & 0b11;
        let tvec_base = tvec & !0b11;
        match tvec_mode { // DIrect
            0 => self.pc = tvec_base,
            1 => self.pc = tvec_base + cause << 2,
            _ => unreachable!(),
        };
        // 3.1.14 & 4.1.7
        // When a trap is taken into S-mode (or M-mode), sepc (or mepc) is written with the virtual address 
        // of the instruction that was interrupted or that encountered the exception.
        self.csr.store(EPC, pc);
        // 3.1.15 & 4.1.8
        // When a trap is taken into S-mode (or M-mode), scause (or mcause) is written with a code indicating 
        // the event that caused the trap.
        self.csr.store(CAUSE, cause);
        // 3.1.16 & 4.1.9
        // When a trap is taken into M-mode, mtval is either set to zero or written with exception-specific 
        // information to assist software in handling the trap. 
        self.csr.store(TVAL, 0);
        // 3.1.6 covers both sstatus and mstatus.
        let mut status = self.csr.load(STATUS);
        // get SIE or MIE
        let ie = (status & MASK_IE) >> ie_i;
        // set SPIE = SIE / MPIE = MIE
        status |= ie << pie_i;
        // set SIE = 0 / MIE = 0
        status &= !MASK_IE; 
        // set SPP / MPP = previous mode
        status = (status & !MASK_PP) | (mode << pp_i);
        self.csr.store(STATUS, status);
    }

    pub fn check_pending_interrupts(&mut self) -> Option<RvIntrrupt> {
        // 3.1.6.1
        // When a hart is executing in privilege mode x, interrupts are globally enabled when x IE=1 and globally 
        // disabled when xIE=0. Interrupts for lower-privilege modes, w<x, are always globally disabled regardless 
        // of the setting of any global wIE bit for the lower-privilege mode. Interrupts for higher-privilege modes, 
        // y>x, are always globally enabled regardless of the setting of the global yIE bit for the higher-privilege 
        // mode. Higher-privilege-level code can use separate per-interrupt enable bits to disable selected higher-
        // privilege-mode interrupts before ceding control to a lower-privilege mode
 
        // 3.1.9 & 4.1.3
        // An interrupt i will trap to M-mode (causing the privilege mode to change to M-mode) if all of
        // the following are true: (a) either the current privilege mode is M and the MIE bit in the mstatus
        // register is set, or the current privilege mode has less privilege than M-mode; (b) bit i is set in both
        // mip and mie; and (c) if register mideleg exists, bit i is not set in mideleg.
        if (self.mode == Machine) && (self.csr.load(MSTATUS) & MASK_MIE) == 0 {
            return None;
        }
        if (self.mode == Supervisor) && (self.csr.load(SSTATUS) & MASK_SIE) == 0 {
            return None;
        }
       
        // 3.1.9 & 4.1.3
        // Multiple simultaneous interrupts destined for M-mode are handled in the following decreasing
        // priority order: MEI, MSI, MTI, SEI, SSI, STI.
        let irq = 


    }

    #[inline]
    pub fn update_pc(&mut self) -> Result<(), RvException> {
        self.pc += 4;
        return Ok(());
    }

    pub fn execute(&mut self, inst: u64) -> Result<(), RvException> {
        let opcode = inst & 0x7f;
        let rd = ((inst >> 7) & 0x1f) as usize;
        let rs1 = ((inst >> 15) & 0x1f) as usize;
        let rs2 = ((inst >> 20) & 0x1f) as usize;
        let funct3 = (inst >> 12) & 0x7;
        let funct7 = (inst >> 25) & 0x7f;

        // x0 is hardwired zero
        self.regs[0] = 0;

        match opcode {
            0x03 => {
                let imm = ((inst as i32 as i64) >> 20) as u64;
                let addr = self.regs[rs1].wrapping_add(imm);
                match funct3 {
                    0x0 => {        // lb
                        let val = self.load(addr, 8)?;
                        self.regs[rd] = val as i8 as i64 as u64;
                        return self.update_pc();
                    }
                    0x1 => {        // lh
                        let val = self.load(addr, 16)?;
                        self.regs[rd] = val as i16 as i64 as u64;
                        return self.update_pc();
                    }
                    0x2 => {        // lw
                        let val = self.load(addr, 32)?;
                        self.regs[rd] = val as i32 as i64 as u64;
                        return self.update_pc();
                    }
                    0x3 => {        // ld
                        let val = self.load(addr, 64)?;
                        self.regs[rd] = val;
                        return self.update_pc();
                    }
                    0x4 => {        // lbu
                        let val = self.load(addr, 8)?;
                        self.regs[rd] = val;
                        return self.update_pc();
                    }
                    0x5 => {        // lhu
                        let val = self.load(addr, 16)?;
                        self.regs[rd] = val;
                        return self.update_pc();
                    }
                    0x6 => {        // lwu
                        let val = self.load(addr, 32)?;
                        self.regs[rd] = val;
                        return self.update_pc();
                    }
                    _ => Err(IllegalInstruction(inst)),
                    
                }
            }
        
            0x13 => {
                // imm[11:0] = inst[31:20]
                let imm = ((inst & 0xfff00000) as i32 as i64 >> 20) as u64;
                // "The shift amount is encoded in the lower 6 bits of the I-immediate field for RV64I."
                let shamt = (imm & 0x3f) as u32;
                match funct3 {
                    0x0 => {
                        // addi
                        self.regs[rd] = self.regs[rs1].wrapping_add(imm);
                        return self.update_pc();
                    }
                    0x1 => {
                        // slli
                        self.regs[rd] = self.regs[rs1] << shamt;
                        return self.update_pc();
                    }
                    0x2 => {
                        // slti
                        self.regs[rd] = if (self.regs[rs1] as i64) < (imm as i64) { 1 } else { 0 };
                        return self.update_pc();
                    }
                    0x3 => {
                        // sltiu
                        self.regs[rd] = if self.regs[rs1] < imm { 1 } else { 0 };
                        return self.update_pc();
                    }
                    0x4 => {
                        // xori
                        self.regs[rd] = self.regs[rs1] ^ imm;
                        return self.update_pc();
                    }
                    0x5 => match funct7 >> 1 {
                        // srli
                        0x00 => {
                            self.regs[rd] = self.regs[rs1].wrapping_shr(shamt);
                            return self.update_pc();
                        }
                        // srai
                        0x10 => {
                            self.regs[rd] = (self.regs[rs1] as i64).wrapping_shr(shamt) as u64;
                            return self.update_pc();
                        }
                        _ => Err(IllegalInstruction(inst)),
                    }
                    0x6 => {
                        self.regs[rd] = self.regs[rs1] | imm; // ori
                        return self.update_pc();
                    }
                    0x7 => {
                        self.regs[rd] = self.regs[rs1] & imm; // andi
                        return self.update_pc();
                    }
                    _ => Err(IllegalInstruction(inst)),
                }
            }
            0x17 => {
                // auipc
                let imm = (inst & 0xfffff000) as i32 as i64 as u64;
                self.regs[rd] = self.pc.wrapping_add(imm);
                return self.update_pc();
            }
            0x1b => {
                let imm = ((inst as i32 as i64) >> 20) as u64;
                // "SLLIW, SRLIW, and SRAIW encodings with imm[5] ̸= 0 are reserved."
                let shamt = (imm & 0x1f) as u32;
                match funct3 {
                    0x0 => {
                        // addiw
                        self.regs[rd] = self.regs[rs1].wrapping_add(imm) as i32 as i64 as u64;
                        return self.update_pc();
                    }
                    0x1 => {
                        // slliw
                        self.regs[rd] = self.regs[rs1].wrapping_shl(shamt) as i32 as i64 as u64;
                        return self.update_pc();
                    }
                    0x5 => {
                        match funct7 {
                            0x00 => {
                                // srliw
                                self.regs[rd] = (self.regs[rs1] as u32).wrapping_shr(shamt) as i32 as i64 as u64;
                                return self.update_pc();
                            }
                            0x20 => {
                                // sraiw
                                self.regs[rd] = (self.regs[rs1] as i32).wrapping_shr(shamt) as i64 as u64;
                                return self.update_pc();
                            }
                            _ => Err(IllegalInstruction(inst)),
                        }
                    }
                    _ => Err(IllegalInstruction(inst)),
                }
            }
            0x23 => {
                let imm = ((inst & 0xfe00_0000) as i32 as i64 >> 20) as u64 | ((inst >> 7) & 0x1f) as u64;
                let addr = self.regs[rs1].wrapping_add(imm);
                match funct3 {
                    0x0 => { self.store(addr, 8, self.regs[rs2])?; self.update_pc() }        // sb
                    0x1 => { self.store(addr, 16, self.regs[rs2])?; self.update_pc() }       // sh
                    0x2 => { self.store(addr, 32, self.regs[rs2])?; self.update_pc() }       // sw
                    0x3 => { self.store(addr, 64, self.regs[rs2])?; self.update_pc() }       // sd
                    _ => Err(IllegalInstruction(inst)),
                }
            }
            0x33 => {
                // "SLL, SRL, and SRA perform logical left, logical right, and arithmetic right
                // shifts on the value in register rs1 by the shift amount held in register rs2.
                // In RV64I, only the low 6 bits of rs2 are considered for the shift amount."
                let shamt = ((self.regs[rs2] & 0x3f) as u64) as u32;
                match (funct3, funct7) {
                    (0x0, 0x00) => {
                        // add
                        self.regs[rd] = self.regs[rs1].wrapping_add(self.regs[rs2]);
                        return self.update_pc();
                    }
                    (0x0, 0x01) => {
                        // mul
                        self.regs[rd] = self.regs[rs1].wrapping_mul(self.regs[rs2]);
                        return self.update_pc();
                    }
                    (0x0, 0x20) => {
                        // sub
                        self.regs[rd] = self.regs[rs1].wrapping_sub(self.regs[rs2]);
                        return self.update_pc();
                    }
                    (0x1, 0x00) => {
                        // sll
                        self.regs[rd] = self.regs[rs1].wrapping_shl(shamt);
                        return self.update_pc();
                    }
                    (0x2, 0x00) => {
                        // slt
                        self.regs[rd] = if (self.regs[rs1] as i64) < (self.regs[rs2] as i64) { 1 } else { 0 };
                        return self.update_pc();
                    }
                    (0x3, 0x00) => {
                        // sltu
                        self.regs[rd] = if self.regs[rs1] < self.regs[rs2] { 1 } else { 0 };
                        return self.update_pc();
                    }
                    (0x4, 0x00) => {
                        // xor
                        self.regs[rd] = self.regs[rs1] ^ self.regs[rs2];
                        return self.update_pc();
                    }
                    (0x5, 0x00) => {
                        // srl
                        self.regs[rd] = self.regs[rs1].wrapping_shr(shamt);
                        return self.update_pc();
                    }
                    (0x5, 0x20) => {
                        // sra
                        self.regs[rd] = (self.regs[rs1] as i64).wrapping_shr(shamt) as u64;
                        return self.update_pc();
                    }
                    (0x6, 0x00) => {
                        // or
                        self.regs[rd] = self.regs[rs1] | self.regs[rs2];
                        return self.update_pc();
                    }
                    (0x7, 0x00) => {
                        // and
                        self.regs[rd] = self.regs[rs1] & self.regs[rs2];
                        return self.update_pc();
                    }
                    _ => Err(IllegalInstruction(inst)),
                }
            }
            0x37 => {
                // lui
                self.regs[rd] = (inst & 0xfffff000) as i32 as i64 as u64;
                return self.update_pc();
            }
            0x3b => {
                // "The shift amount is given by rs2[4:0]."
                let shamt = (self.regs[rs2] & 0x1f) as u32;
                match (funct3, funct7) {
                    (0x0, 0x00) => {
                        // addw
                        self.regs[rd] = self.regs[rs1].wrapping_add(self.regs[rs2]) as i32 as i64 as u64;
                        return self.update_pc();
                    }
                    (0x0, 0x20) => {
                        // subw
                        self.regs[rd] = ((self.regs[rs1].wrapping_sub(self.regs[rs2])) as i32) as u64;
                        return self.update_pc();
                    }
                    (0x1, 0x00) => {
                        // sllw
                        self.regs[rd] = (self.regs[rs1] as u32).wrapping_shl(shamt) as i32 as u64;
                        return self.update_pc();
                    }
                    (0x5, 0x00) => {
                        // srlw
                        self.regs[rd] = (self.regs[rs1] as u32).wrapping_shr(shamt) as i32 as u64;
                        return self.update_pc();
                    }
                    (0x5, 0x20) => {
                        // sraw
                        self.regs[rd] = ((self.regs[rs1] as i32) >> (shamt as i32)) as u64;
                        return self.update_pc();
                    }
                    _ => Err(IllegalInstruction(inst)), 
                }
            }
            0x63 => {
                // imm[12|10:5|4:1|11] = inst[31|30:25|11:8|7]
                let imm = (((inst & 0x80000000) as i32 as i64 >> 19) as u64)
                    | ((inst & 0x80) << 4) as u64// imm[11]
                    | ((inst >> 20) & 0x7e0) as u64// imm[10:5]
                    | ((inst >> 7) & 0x1e) as u64; // imm[4:1]

                match funct3 {
                    0x0 => {
                        // beq
                        if self.regs[rs1] == self.regs[rs2] {
                            self.pc = self.pc.wrapping_add(imm);
                            return Ok(());
                        } 
                        return self.update_pc();
                    }
                    0x1 => {
                        // bne
                        if self.regs[rs1] != self.regs[rs2] {
                            self.pc = self.pc.wrapping_add(imm);
                            return Ok(());
                        }
                        return self.update_pc();
                    }
                    0x4 => {
                        // blt
                        if (self.regs[rs1] as i64) < (self.regs[rs2] as i64) {
                            self.pc = self.pc.wrapping_add(imm);
                            return Ok(());
                        }
                        return self.update_pc();
                    }
                    0x5 => {
                        // bge
                        if (self.regs[rs1] as i64) >= (self.regs[rs2] as i64) {
                            self.pc = self.pc.wrapping_add(imm);
                            return Ok(());
                        }
                        return self.update_pc();
                    }
                    0x6 => {
                        // bltu
                        if self.regs[rs1] < self.regs[rs2] {
                            self.pc = self.pc.wrapping_add(imm);
                            return Ok(());
                        }
                        return self.update_pc();
                    }
                    0x7 => {
                        // bgeu
                        if self.regs[rs1] >= self.regs[rs2] {
                            self.pc = self.pc.wrapping_add(imm);
                            return Ok(());
                        }
                        return self.update_pc();
                    }
                    _ => Err(IllegalInstruction(inst)),
                }
            }
            0x67 => {
                // jalr
                let t = self.pc + 4;
                let imm = ((((inst & 0xfff00000) as i32) as i64) >> 20) as u64;
                self.pc = (self.regs[rs1].wrapping_add(imm)) & !1;
                self.regs[rd] = t;
                return Ok(());
            }
            0x6f => {
                // jal
                self.regs[rd] = self.pc + 4;
                // imm[20|10:1|11|19:12] = inst[31|30:21|20|19:12]
                let imm = (((inst & 0x80000000) as i32 as i64 >> 11) as u64) // imm[20]
                    | (inst & 0xff000)  as u64// imm[19:12]
                    | ((inst >> 9) & 0x800) as u64// imm[11]
                    | ((inst >> 20) & 0x7fe) as u64; // imm[10:1]
                self.pc = self.pc.wrapping_add(imm);
                return Ok(());
            }
            0x73 => {
                let csr_addr = ((inst & 0xfff00000) >> 20) as usize;
                match funct3 {
                    0x0 => {
                        match (rs2, funct7) {
                            // ECALL and EBREAK cause the receiving privilege mode’s epc register to be set to the address of
                            // the ECALL or EBREAK instruction itself, not the address of the following instruction.
                            (0x0, 0x0) => {
                                // ecall
                                match self.mode {
                                    User => Err(RvException::EnvironmentCallFromUmode(self.pc)),
                                    Supervisor => Err(RvException::EnvironmentCallFromSmode(self.pc)),
                                    Machine => Err(RvException::EnvironmentCallFromMmode(self.pc)),
                                    _ => unreachable!(),
                                }
                            }
                            (0x1, 0x0) => {
                                // ebreak
                                return Err(RvException::Breakpoint(self.pc));
                            }
                            (0x2, 0x8) => {
                                // sret
                                // set the pc to CSRs[sepc].
                                self.pc = self.csr.load(SEPC);
                                // When the SRET instruction is executed to return from the trap
                                // handler, the privilege level is set to user mode if the SPP
                                // bit is 0, or supervisor mode if the SPP bit is 1. The SPP bit
                                // is SSTATUS[8].
                                let mut sstatus = self.csr.load(SSTATUS);
                                self.mode = (sstatus & MASK_SPP) >> 8;
                                // The SPIE bit is SSTATUS[5] and the SIE bit is the SSTATUS[1]
                                let spie = (sstatus & MASK_SPIE) >> 5;
                                // set SIE = SPIE
                                sstatus |= spie << 1;
                                // set SPIE = 1
                                sstatus |= MASK_SPIE;
                                // set SPP the least privilege mode (u-mode)
                                sstatus &= !MASK_SPP;
                                self.csr.store(SSTATUS, sstatus);
                                return Ok(());
                            }
                            (0x2, 0x18) => {
                                // mret
                                // set the pc to CSRs[mepc].
                                self.pc = self.csr.load(MEPC);
                                let mut mstatus = self.csr.load(MSTATUS);
                                // MPP is two bits wide at MSTATUS[12:11]
                                self.mode = (mstatus & MASK_MPP) >> 11;
                                // The MPIE bit is MSTATUS[7] and the MIE bit is the MSTATUS[3].
                                let mpie = (mstatus >> 7) & 1;
                                // set MIE = MPIE
                                mstatus |= mpie << 3;
                                // set MPIE = 1
                                mstatus |= MASK_MPIE;
                                // set MPP the least privilege mode (u-mode)
                                mstatus &= !MASK_MPP;
                                self.csr.store(MSTATUS, mstatus);
                                return Ok(());
                            }
                            (_, 0x9) => {
                                // sfence.vma
                                // Do nothing.
                                return Ok(());
                            }
                            _ => Err(IllegalInstruction(inst)),
                        }
                    }
                    0x1 => {
                        // csrrw
                        let t = self.csr.load(csr_addr);
                        self.csr.store(csr_addr, self.regs[rs1]);
                        self.regs[rd] = t;
                        return self.update_pc();
                    }
                    0x2 => {
                        // csrrs
                        let t = self.csr.load(csr_addr);
                        self.csr.store(csr_addr, t | self.regs[rs1]);
                        self.regs[rd] = t;
                        return self.update_pc();
                    }
                    0x3 => {
                        // csrrc
                        let t = self.csr.load(csr_addr);
                        self.csr.store(csr_addr, t & (!self.regs[rs1]));
                        self.regs[rd] = t;
                        return self.update_pc();
                    }
                    0x5 => {
                        // csrrwi
                        let zimm = rs1 as u64;
                        self.regs[rd] = self.csr.load(csr_addr);
                        self.csr.store(csr_addr, zimm);
                        return self.update_pc();
                    }
                    0x6 => {
                        // csrrsi
                        let zimm = rs1 as u64;
                        let t = self.csr.load(csr_addr);
                        self.csr.store(csr_addr, t | zimm);
                        self.regs[rd] = t;
                        return self.update_pc();
                    }
                    0x7 => {
                        // csrrci
                        let zimm = rs1 as u64;
                        let t = self.csr.load(csr_addr);
                        self.csr.store(csr_addr, t & (!zimm));
                        self.regs[rd] = t;
                        return self.update_pc();
                    }
                    _ => Err(IllegalInstruction(inst)),
                }
            }
            _ => Err(IllegalInstruction(inst)),
        }
    }
}


#[cfg(test)]
mod test {
    use std::fs::File;
    use std::io::{Write, Read};
    use std::process::Command;
    use super::*;

    fn generate_rv_assembly(c_src: &str) {
        let cc = "clang";
        let output = Command::new(cc).arg("-S")
                            .arg(c_src)
                            .arg("-nostdlib")
                            .arg("-march=rv64g")
                            .arg("-mabi=lp64")
                            .arg("--target=riscv64")
                            .arg("-mno-relax")
                            .output()
                            .expect("Failed to generate rv assembly");
        println!("{}", String::from_utf8_lossy(&output.stderr));
    }

    fn generate_rv_obj(assembly: &str) {
        let cc = "clang";
        let pieces: Vec<&str> = assembly.split(".").collect();
        let output = Command::new(cc).arg("-Wl,-Ttext=0x0")
                            .arg("-nostdlib")
                            .arg("-march=rv64g")
                            .arg("-mabi=lp64")
                            .arg("--target=riscv64")
                            .arg("-mno-relax")
                            .arg("-o")
                            .arg(&pieces[0])
                            .arg(assembly)
                            .output()
                            .expect("Failed to generate rv object");
        println!("{}", String::from_utf8_lossy(&output.stderr));
    }

    fn generate_rv_binary(obj: &str) {
        let objcopy = "llvm-objcopy";
        let output = Command::new(objcopy).arg("-O")
                                .arg("binary")
                                .arg(obj)
                                .arg(obj.to_owned() + ".bin")
                                .output()
                                .expect("Failed to generate rv binary");
        println!("{}", String::from_utf8_lossy(&output.stderr));
    }

    fn rv_helper(code: &str, testname: &str, n_clock: usize) -> Result<Cpu, std::io::Error> {
        let filename = testname.to_owned() + ".s";
        let mut file = File::create(&filename)?;
        file.write(&code.as_bytes())?;
        generate_rv_obj(&filename);
        generate_rv_binary(testname);
        let mut file_bin = File::open(testname.to_owned() + ".bin")?;
        let mut code = Vec::new();
        file_bin.read_to_end(&mut code)?;
        let mut cpu = Cpu::new(code);

        for _i in 0..n_clock {
            let inst = match cpu.fetch() {
                Ok(inst) => inst,
                Err(_err) => break,
            };
            match cpu.execute(inst) {
                Ok(_) => (),
                Err(err) => println!("{}", err),
            };
        }

        return Ok(cpu);
    }

    macro_rules! riscv_test {
        ( $code:expr, $name:expr, $clock:expr, $($real:expr => $expect:expr),* ) => {
            match rv_helper($code, $name, $clock) {
                Ok(cpu) => { 
                    $(assert_eq!(cpu.reg($real), $expect);)*
                }
                Err(e) => { println!("error: {}", e); assert!(false); }
            } 
        };
    }

    #[test]
    fn test_addi() {
        let code = "addi x31, x0, 42";
        riscv_test!(code, "test_addi", 1, "x31" => 42);
    }

    #[test]
    fn test_simple() {
        // this is the assembly code of simple.c
        let code = "
            addi	sp,sp,-16
            sd	s0,8(sp)
            addi	s0,sp,16
            li	a5,42
            mv	a0,a5
            ld	s0,8(sp)
            addi	sp,sp,16
            jr	ra
        ";
        riscv_test!(code, "test_simple", 20, "a0" => 42);
    }

    #[test]
    fn test_lui() {
        let code = "lui a0, 42";
        riscv_test!(code, "test_lui", 1, "a0" => 42 << 12);
    }

    #[test]
    fn test_auipc() {
        let code = "auipc a0, 42";
        riscv_test!(code, "test_auipc", 1, "a0" => DRAM_BASE + (42 << 12));
    }

    #[test]
    fn test_jal() {
        let code = "jal a0, 42";
        riscv_test!(code, "test_jal", 1, "a0" => DRAM_BASE + 4, "pc" => DRAM_BASE + 42);
    }

    #[test]
    fn test_jalr() {
        let code = "
            addi a1, zero, 42
            jalr a0, -8(a1)
        ";
        riscv_test!(code, "test_jalr", 2, "a0" => DRAM_BASE + 8, "pc" => 34);
    }

    #[test]
    fn test_beq() {
        let code = "
            beq  x0, x0, 42
        ";
        riscv_test!(code, "test_beq", 3, "pc" => DRAM_BASE + 42);
    }

    #[test]
    fn test_bne() {
        let code = "
            addi x1, x0, 10
            bne  x0, x1, 42
        ";
        riscv_test!(code, "test_bne", 5, "pc" => DRAM_BASE + 42 + 4);
    }

    #[test]
    fn test_blt() {
        let code = "
            addi x1, x0, 10
            addi x2, x0, 20
            blt  x1, x2, 42
        ";
        riscv_test!(code, "test_blt", 10, "pc" => DRAM_BASE + 42 + 8);
    }

    #[test]
    fn test_bge() {
        let code = "
            addi x1, x0, 10
            addi x2, x0, 20
            bge  x2, x1, 42
        ";
        riscv_test!(code, "test_bge", 10, "pc" => DRAM_BASE + 42 + 8);
    }

    #[test]
    fn test_bltu() {
        let code = "
            addi x1, x0, 10
            addi x2, x0, 20
            bltu x1, x2, 42
        ";
        riscv_test!(code, "test_bltu", 10, "pc" => DRAM_BASE + 42 + 8);
    }

    #[test]
    fn test_bgeu() {
        let code = "
            addi x1, x0, 10
            addi x2, x0, 20
            bgeu x2, x1, 42
        ";
        riscv_test!(code, "test_bgeu", 10, "pc" => DRAM_BASE + 42 + 8);
    }

    #[test]
    fn test_store_load1() {
        let code = "
            addi s0, zero, 256
            addi sp, sp, -16
            sd   s0, 8(sp)
            lb   t1, 8(sp)
            lh   t2, 8(sp)
        ";
        riscv_test!(code, "test_store_load1", 10, "t1" => 0, "t2" => 256);
    }

    #[test]
    fn test_slt() {
        let code = "
            addi t0, zero, 14
            addi t1, zero, 24
            slt  t2, t0, t1
            slti t3, t0, 42
            sltiu t4, t0, 84
        ";
        riscv_test!(code, "test_slt", 7, "t2" => 1, "t3" => 1, "t4" => 1);
    }

    #[test]
    fn test_xor() {
        let code = "
            addi a0, zero, 0b10
            xori a1, a0, 0b01
            xor a2, a1, a1 
        ";
        riscv_test!(code, "test_xor", 5, "a1" => 3, "a2" => 0);
    }

    #[test]
    fn test_or() {
        let code = "
            addi a0, zero, 0b10
            ori  a1, a0, 0b01
            or   a2, a0, a0
        ";
        riscv_test!(code, "test_or", 3, "a1" => 0b11, "a2" => 0b10);
    }

    #[test]
    fn test_and() {
        let code = "
            addi a0, zero, 0b10 
            andi a1, a0, 0b11
            and  a2, a0, a1
        ";
        riscv_test!(code, "test_and", 3, "a1" => 0b10, "a2" => 0b10);
    }

    #[test]
    fn test_sll() {
        let code = "
            addi a0, zero, 1
            addi a1, zero, 5
            sll  a2, a0, a1
            slli a3, a0, 5
            addi s0, zero, 64
            sll  a4, a0, s0
        ";
        riscv_test!(code, "test_sll", 10, "a2" => 1 << 5, "a3" => 1 << 5, "a4" => 1);
    }

    #[test]
    fn test_sra_srl() {
        let code = "
            addi a0, zero, -8
            addi a1, zero, 1
            sra  a2, a0, a1
            srai a3, a0, 2
            srli a4, a0, 2
            srl  a5, a0, a1
        ";
        riscv_test!(code, "test_sra_srl", 10, "a2" => -4 as i64 as u64, "a3" => -2 as i64 as u64, 
                                              "a4" => -8 as i64 as u64 >> 2, "a5" => -8 as i64 as u64 >> 1);
    }

    #[test]
    fn test_word_op() {
        let code = "
            addi a0, zero, 42 
            lui  a1, 0x7f000
            addw a2, a0, a1
        ";
        riscv_test!(code, "test_word_op", 29, "a2" => 0x7f00002a);
    }

    #[test]
    fn test_csrs1() {
        let code = "
            addi t0, zero, 1
            addi t1, zero, 2
            addi t2, zero, 3
            csrrw zero, mstatus, t0
            csrrs zero, mtvec, t1
            csrrw zero, mepc, t2
            csrrc t2, mepc, zero
            csrrwi zero, sstatus, 4
            csrrsi zero, stvec, 5
            csrrwi zero, sepc, 6
            csrrci zero, sepc, 0 
        ";
        riscv_test!(code, "test_csrs1", 20, "mstatus" => 1, "mtvec" => 2, "mepc" => 3,
                                            "sstatus" => 0, "stvec" => 5, "sepc" => 6);
    }

    #[test]
    fn compile_hello_world() {
        // You should run it by
        // -- cargo run helloworld.bin
        let c_code = r"
        int main() {
            volatile char *uart = (volatile char *) 0x10000000;
            uart[0] = 'H';
            uart[0] = 'e';
            uart[0] = 'l';
            uart[0] = 'l';
            uart[0] = 'o';
            uart[0] = ',';
            uart[0] = ' ';
            uart[0] = 'w';
            uart[0] = 'o';
            uart[0] = 'r';
            uart[0] = 'l';
            uart[0] = 'd';
            uart[0] = '!';
            uart[0] = '\n';
            return 0;
        }";
        let mut file = File::create("test_helloworld.c").unwrap();
        file.write(&c_code.as_bytes()).unwrap();
        generate_rv_assembly("test_helloworld.c");
        generate_rv_obj("test_helloworld.s");
        generate_rv_binary("test_helloworld");
    }

    #[test]
    fn compile_echoback() {
        let c_code = r"
        int main() {
            while (1) {
                volatile char *uart = (volatile char *) 0x10000000;
                while ((uart[5] & 0x01) == 0);
                char c = uart[0];
                if ('a' <= c && c <= 'z') {
                    c = c + 'A' - 'a';
                }
                uart[0] = c;
            }
        }";
        let mut file = File::create("test_echoback.c").unwrap();
        file.write(&c_code.as_bytes()).unwrap();
        generate_rv_assembly("test_echoback.c");
        generate_rv_obj("test_echoback.s");
        generate_rv_binary("test_echoback");
    }
}