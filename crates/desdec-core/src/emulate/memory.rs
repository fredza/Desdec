//! The memory the emulated processor sees.
//!
//! Nothing here is the machine's own memory. The address space is built from
//! the file's own section table — each mapped section at the address its
//! headers claim, with the rights they claim — plus a stack the emulator
//! allocates because no file carries one. A page that no section covers is not
//! zero: it is *absent*, and reading it stops the run instead of yielding a
//! plausible zero that would make a null dereference look like ordinary work.
//!
//! The file's bytes are never copied. They are borrowed as the initial content
//! of every region backed by the file, and a write puts the changed page in an
//! overlay that is consulted first. Emulating a shared library therefore costs
//! the pages the program actually wrote to, not the hundred megabytes it was
//! read from.
//!
//! Rights are enforced rather than recorded. Writing to `.text` faults, and so
//! does fetching an instruction from `.data`: those are the two mistakes worth
//! catching, and a reader who sees the fault has learnt something the listing
//! alone does not say.

use std::{collections::BTreeMap, sync::Arc};

use crate::analysis::{Analysis, Permissions, Section, details::Segment};

/// Bytes to a page. The value every architecture Desdec decodes agrees on,
/// and the granularity at which rights are enforced.
pub const PAGE: u64 = 4096;

/// The first page is never mapped, whatever the file says.
///
/// Every operating system leaves it out for the same reason: a program that
/// follows a null pointer must fault rather than read something. It matters
/// more here than anywhere, because a file analysed on its own has no loader
/// to fill in its table of external calls — every entry of it still reads
/// zero, and an indirect call through one lands on the first page. Mapping it
/// would send the run wandering through the file's own headers, decoding them
/// as if they were code; leaving it out says plainly that the call went
/// somewhere no loader has filled in.
const FIRST_PAGE_KEPT_ABSENT: u64 = PAGE;

/// Where the emulator puts the stack when the file does not say.
///
/// Chosen far above where a linker maps an executable and far below where a
/// kernel maps itself, so it collides with neither. It is only a default: a
/// region already covering it wins, and the stack is put elsewhere.
pub const DEFAULT_STACK_TOP: u64 = 0x0000_7fff_ffff_0000;

/// How much room the stack is given. A megabyte is what a thread gets on
/// Windows by default, and enough for any recursion worth stepping through.
pub const DEFAULT_STACK_SIZE: u64 = 1024 * 1024;

/// Why a memory access could not be carried out.
///
/// Each one stops the run and is reported in the reader's language. None of
/// them is recoverable: the emulator has no operating system behind it to
/// handle a fault, and pretending otherwise would invent a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    /// No section and no allocation covers the address.
    Unmapped { address: u64 },
    /// The address is mapped, but not with the right the access needed.
    Protection {
        address: u64,
        needed: Access,
        granted: Permissions,
    },
}

impl Fault {
    /// The address the access could not be carried out at.
    #[must_use]
    pub const fn address(self) -> u64 {
        match self {
            Self::Unmapped { address } | Self::Protection { address, .. } => address,
        }
    }
}

/// What an access needs the page to allow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    Read,
    Write,
    Execute,
}

impl Access {
    /// Whether a set of rights permits this access.
    const fn permitted_by(self, permissions: Permissions) -> bool {
        match self {
            Self::Read => permissions.read,
            Self::Write => permissions.write,
            Self::Execute => permissions.execute,
        }
    }
}

/// One mapped run of addresses, and where its initial bytes come from.
///
/// The three `file_*` fields say the same thing a program header says: *these*
/// bytes of the file go *there* in the region, and everything else in it is
/// zero. A `.bss` has none of them, and so does a stack.
#[derive(Clone, Debug)]
pub struct Region {
    /// What to call it on screen. A section's name, or the emulator's own.
    pub name: String,
    pub start: u64,
    pub size: u64,
    pub permissions: Permissions,
    /// Where inside the region the file's bytes begin. Non-zero when the
    /// section did not start on a page boundary.
    data_at: u64,
    /// Where those bytes are in the file.
    file_offset: u64,
    /// How many of them there are. Zero for a region the file does not back.
    file_size: u64,
}

impl Region {
    /// The first address past the region.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.start.wrapping_add(self.size)
    }

    /// Whether an address falls inside it.
    #[must_use]
    pub const fn contains(&self, address: u64) -> bool {
        address >= self.start && address.wrapping_sub(self.start) < self.size
    }
}

/// The emulated address space.
#[derive(Clone)]
pub struct Memory {
    /// The file, exactly as it is on disk. Shared, never modified.
    file: Arc<[u8]>,
    /// Mapped regions, sorted by address and never overlapping.
    regions: Vec<Region>,
    /// Pages that have been written to, which win over the file's bytes.
    overlay: BTreeMap<u64, Box<[u8]>>,
}

impl Memory {
    /// An address space with nothing in it, over the given file image.
    #[must_use]
    pub fn new(file: Arc<[u8]>) -> Self {
        Self {
            file,
            regions: Vec::new(),
            overlay: BTreeMap::new(),
        }
    }

    /// Builds the address space the binary itself describes.
    ///
    /// A loader maps *segments*, not sections: an ELF program header or a
    /// Mach-O segment says which bytes go where and with what rights, and the
    /// sections inside one are a finer division of the same bytes that no
    /// loader ever consults. Mapping by section instead puts `.text` and the
    /// note before it in the same page under the note's read-only rights, and
    /// the first instruction fetched then faults.
    ///
    /// PE is the exception, and the one case where the sections *are* the
    /// mapping: its headers carry no separate segment table, so they are used.
    #[must_use]
    pub fn load(file: Arc<[u8]>, analysis: &Analysis) -> Self {
        let mut memory = Self::new(file);
        if analysis.details.segments.is_empty() {
            let mut mapped: Vec<&Section> = analysis
                .sections
                .iter()
                .filter(|section| section.is_mapped())
                .collect();
            mapped.sort_by_key(|section| section.virtual_address);
            for section in mapped {
                memory.map(
                    &section.name,
                    section.virtual_address,
                    section.virtual_size,
                    section.file_offset,
                    section.file_size,
                    section.permissions,
                );
            }
            return memory;
        }
        let mut segments: Vec<&Segment> = analysis
            .details
            .segments
            .iter()
            .filter(|segment| segment.virtual_size > 0)
            .collect();
        segments.sort_by_key(|segment| segment.virtual_address);
        for segment in segments {
            memory.map(
                &segment.kind,
                segment.virtual_address,
                segment.virtual_size,
                segment.file_offset,
                segment.file_size,
                segment.permissions,
            );
        }
        memory
    }

    /// Builds the address space a section table describes, for a caller that
    /// has sections and nothing else — the tests, and a format with no
    /// segments of its own.
    #[must_use]
    pub fn from_sections(file: Arc<[u8]>, sections: &[Section]) -> Self {
        let mut memory = Self::new(file);
        let mut mapped: Vec<&Section> = sections.iter().filter(|s| s.is_mapped()).collect();
        mapped.sort_by_key(|section| section.virtual_address);
        for section in mapped {
            memory.map(
                &section.name,
                section.virtual_address,
                section.virtual_size,
                section.file_offset,
                section.file_size,
                section.permissions,
            );
        }
        memory
    }

    /// Maps one run of the file, page by page, over whatever is free.
    ///
    /// Two mappings can land in one page — a linker is free to put the end of
    /// one segment and the start of the next inside the same four kilobytes —
    /// and the two rules that settles are stated here rather than left to the
    /// order the table happened to be in:
    ///
    /// - **The rights of a shared page are the union of both.** A page holding
    ///   the tail of the code and the head of the data is executable *and*
    ///   writable, which is what a loader leaves behind, and refusing either
    ///   would fault on a fetch or on a store that a real run makes.
    /// - **The bytes of a shared page are the first mapping's.** They are the
    ///   same bytes either way whenever the file is well formed; when it is
    ///   not, one answer has to be chosen, and the earlier address is the one
    ///   the reader is more likely to be looking at.
    fn map(
        &mut self,
        name: &str,
        virtual_address: u64,
        virtual_size: u64,
        file_offset: u64,
        file_size: u64,
        permissions: Permissions,
    ) {
        let start = (virtual_address & !(PAGE - 1)).max(FIRST_PAGE_KEPT_ABSENT);
        let end = round_up(virtual_address.saturating_add(virtual_size));
        let mut at = start;
        while at < end {
            let Some(free) = self.next_free(at, end) else {
                break;
            };
            let (from, to) = free;
            // Where in the file this stretch's bytes are: the request's own
            // correspondence, moved to the stretch's first address.
            let ahead = from.saturating_sub(virtual_address);
            let data_at = virtual_address.saturating_sub(from);
            self.insert(Region {
                name: name.to_owned(),
                start: from,
                size: to - from,
                permissions,
                data_at,
                file_offset: file_offset.saturating_add(ahead),
                file_size: file_size.saturating_sub(ahead),
            });
            at = to;
        }
        // Whatever of the request was already mapped keeps its bytes and gains
        // the rights this mapping asked for.
        for region in &mut self.regions {
            if region.start < end && start < region.end() {
                region.permissions.read |= permissions.read;
                region.permissions.write |= permissions.write;
                region.permissions.execute |= permissions.execute;
            }
        }
    }

    /// The first stretch of `from..limit` that nothing is mapped over.
    fn next_free(&self, from: u64, limit: u64) -> Option<(u64, u64)> {
        let mut at = from;
        while at < limit {
            if let Some(region) = self.region_at(at) {
                at = region.end();
                continue;
            }
            // Free from here to whichever comes first: the next region, or the
            // end of what was asked for.
            let next = self
                .regions
                .iter()
                .map(|region| region.start)
                .find(|start| *start > at)
                .unwrap_or(limit)
                .min(limit);
            return Some((at, next));
        }
        None
    }

    /// Reserves a run of zeroed, readable and writable addresses that the file
    /// does not describe — a stack, a heap, a place to put a return address.
    ///
    /// Returns whether there was room: an allocation that would land on top of
    /// a mapping is refused rather than silently moved, because the caller
    /// chose the address for a reason.
    pub fn allocate(&mut self, name: impl Into<String>, start: u64, size: u64) -> bool {
        self.insert(Region {
            name: name.into(),
            start: start & !(PAGE - 1),
            size: round_up(size),
            permissions: Permissions {
                read: true,
                write: true,
                execute: false,
            },
            data_at: 0,
            file_offset: 0,
            file_size: 0,
        })
    }

    /// Puts a region in the table, keeping it sorted, unless it overlaps.
    fn insert(&mut self, region: Region) -> bool {
        if region.size == 0 || self.overlaps(region.start, region.size) {
            return false;
        }
        let at = self
            .regions
            .partition_point(|other| other.start < region.start);
        self.regions.insert(at, region);
        true
    }

    /// Whether anything is already mapped in `start..start + size`.
    fn overlaps(&self, start: u64, size: u64) -> bool {
        let end = start.wrapping_add(size);
        self.regions
            .iter()
            .any(|region| start < region.end() && region.start < end)
    }

    /// Every mapped region, in address order.
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// The region an address falls in, if any.
    #[must_use]
    pub fn region_at(&self, address: u64) -> Option<&Region> {
        let index = self
            .regions
            .partition_point(|region| region.start <= address)
            .checked_sub(1)?;
        let region = self.regions.get(index)?;
        region.contains(address).then_some(region)
    }

    /// Reads bytes, checking that every one of them may be read.
    ///
    /// An access that crosses a region boundary is checked on both sides: the
    /// rights belong to the page, not to the first byte of the access.
    ///
    /// # Errors
    ///
    /// [`Fault::Unmapped`] if nothing is mapped at one of the addresses, and
    /// [`Fault::Protection`] if one of them may not be read.
    pub fn read(&self, address: u64, into: &mut [u8]) -> Result<(), Fault> {
        self.read_with(address, into, Access::Read)
    }

    /// Reads the bytes an instruction is made of.
    ///
    /// Separate from [`Self::read`] only in the right it asks for: fetching
    /// from a page that is not executable is the mistake this catches.
    ///
    /// # Errors
    ///
    /// As [`Self::read`], with the execute right asked for instead.
    pub fn fetch(&self, address: u64, into: &mut [u8]) -> Result<(), Fault> {
        self.read_with(address, into, Access::Execute)
    }

    fn read_with(&self, address: u64, into: &mut [u8], access: Access) -> Result<(), Fault> {
        let mut at = address;
        for slot in into.iter_mut() {
            self.permit(at, access)?;
            *slot = self.stored(at);
            at = at.wrapping_add(1);
        }
        Ok(())
    }

    /// Reads what is there, with no right required and no fault raised: absent
    /// and unreadable bytes come back as `None`.
    ///
    /// This is what a memory view is for. A reader looking at a dump has not
    /// executed anything, and a page they cannot read should be drawn as
    /// absent rather than end their run.
    #[must_use]
    pub fn peek(&self, address: u64) -> Option<u8> {
        self.region_at(address)?;
        Some(self.stored(address))
    }

    /// Writes bytes, checking that every one of them may be written.
    ///
    /// # Errors
    ///
    /// As [`Self::read`], with the write right asked for instead. Nothing is
    /// stored when it fails.
    pub fn write(&mut self, address: u64, from: &[u8]) -> Result<(), Fault> {
        // Checked in full before anything is stored: a write that faulted
        // half-way would leave memory in a state no execution ever has.
        let mut at = address;
        for _ in from {
            self.permit(at, Access::Write)?;
            at = at.wrapping_add(1);
        }
        let mut at = address;
        for byte in from {
            self.set_byte(at, *byte);
            at = at.wrapping_add(1);
        }
        Ok(())
    }

    /// Writes a byte with no right required, as a reader editing a memory cell
    /// by hand is entitled to do.
    ///
    /// Returns whether the address is mapped at all: even by hand, a byte
    /// cannot be put where there is no memory.
    pub fn poke(&mut self, address: u64, byte: u8) -> bool {
        if self.region_at(address).is_none() {
            return false;
        }
        self.set_byte(address, byte);
        true
    }

    /// Checks an address against the rights of the region holding it.
    fn permit(&self, address: u64, access: Access) -> Result<(), Fault> {
        let Some(region) = self.region_at(address) else {
            return Err(Fault::Unmapped { address });
        };
        if access.permitted_by(region.permissions) {
            Ok(())
        } else {
            Err(Fault::Protection {
                address,
                needed: access,
                granted: region.permissions,
            })
        }
    }

    /// What is at an address: the overlay if it has been written, the file's
    /// bytes if the region is backed by any, and zero otherwise.
    fn stored(&self, address: u64) -> u8 {
        let page = address & !(PAGE - 1);
        if let Some(written) = self.overlay.get(&page) {
            let inside = usize::try_from(address - page).unwrap_or(0);
            if let Some(byte) = written.get(inside) {
                return *byte;
            }
        }
        self.file_byte(address).unwrap_or(0)
    }

    /// The file's own byte for an address, when the region has one there.
    fn file_byte(&self, address: u64) -> Option<u8> {
        let region = self.region_at(address)?;
        let inside = address.wrapping_sub(region.start);
        let within = inside.checked_sub(region.data_at)?;
        if within >= region.file_size {
            return None;
        }
        let at = usize::try_from(region.file_offset.checked_add(within)?).ok()?;
        self.file.get(at).copied()
    }

    /// Stores a byte in the overlay, materialising the page if needed.
    fn set_byte(&mut self, address: u64, byte: u8) {
        let page = address & !(PAGE - 1);
        if !self.overlay.contains_key(&page) {
            // A page enters the overlay holding what it held before, so a
            // one-byte write does not blank the rest of it.
            let mut fresh = vec![0_u8; usize::try_from(PAGE).unwrap_or(4096)];
            let mut at = page;
            for slot in &mut fresh {
                *slot = self.file_byte(at).unwrap_or(0);
                at = at.wrapping_add(1);
            }
            self.overlay.insert(page, fresh.into_boxed_slice());
        }
        let inside = usize::try_from(address - page).unwrap_or(0);
        if let Some(written) = self.overlay.get_mut(&page)
            && let Some(slot) = written.get_mut(inside)
        {
            *slot = byte;
        }
    }

    /// How many pages the run has written to. What the interface reports when
    /// it says how much memory the emulation is holding.
    #[must_use]
    pub fn written_pages(&self) -> usize {
        self.overlay.len()
    }

    /// Forgets every write, leaving the file's own bytes. Used when a run is
    /// restarted: the next run must see the image, not the last run's leavings.
    pub fn forget_writes(&mut self) {
        self.overlay.clear();
    }
}

/// Rounds a size up to a whole number of pages.
const fn round_up(size: u64) -> u64 {
    size.saturating_add(PAGE - 1) & !(PAGE - 1)
}
