//! The declarations of the executable formats themselves.
//!
//! The first structure a reader of a binary wants to lay over it is the one
//! the file starts with. Typing `Elf64_Ehdr` out by hand from the manual page
//! is twenty lines of work that is the same for every ELF ever opened, and
//! getting one field's width wrong puts every field after it at the wrong
//! offset — which is precisely the failure this whole module exists to avoid.
//!
//! So the formats Desdec reads come written down. The declarations are the
//! ones in `elf.h`, in `winnt.h` and in `<mach-o/loader.h>`, spelled with the
//! fixed-width names so that they lay out the same whatever the model.
//!
//! Nothing here is applied on its own. It is offered, and the reader who takes
//! it can edit it like anything else they wrote.

use crate::BinaryFormat;

/// The declarations of `format`, or `None` for one that was not recognised.
#[must_use]
pub const fn of(format: BinaryFormat) -> Option<&'static str> {
    Some(match format {
        BinaryFormat::Elf { bits: 32, .. } => ELF32,
        BinaryFormat::Elf { .. } => ELF64,
        BinaryFormat::Pe => PE,
        BinaryFormat::MachO { bits: 64, .. } => MACH_O,
        // A 32-bit Mach-O and a format that was not recognised are both
        // offered nothing rather than the declarations of something else.
        BinaryFormat::MachO { .. } | BinaryFormat::Unknown => return None,
    })
}

/// The structure the file itself begins with, which is where a reader starts.
#[must_use]
pub const fn header_of(format: BinaryFormat) -> Option<&'static str> {
    Some(match format {
        BinaryFormat::Elf { bits: 32, .. } => "Elf32_Ehdr",
        BinaryFormat::Elf { .. } => "Elf64_Ehdr",
        BinaryFormat::Pe => "IMAGE_DOS_HEADER",
        BinaryFormat::MachO { bits: 64, .. } => "mach_header_64",
        BinaryFormat::MachO { .. } | BinaryFormat::Unknown => return None,
    })
}

/// A 64-bit ELF, as `elf.h` declares it.
const ELF64: &str = "\
struct Elf64_Ehdr {
    unsigned char e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint64_t e_entry;
    uint64_t e_phoff;
    uint64_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
};

struct Elf64_Phdr {
    uint32_t p_type;
    uint32_t p_flags;
    uint64_t p_offset;
    uint64_t p_vaddr;
    uint64_t p_paddr;
    uint64_t p_filesz;
    uint64_t p_memsz;
    uint64_t p_align;
};

struct Elf64_Shdr {
    uint32_t sh_name;
    uint32_t sh_type;
    uint64_t sh_flags;
    uint64_t sh_addr;
    uint64_t sh_offset;
    uint64_t sh_size;
    uint32_t sh_link;
    uint32_t sh_info;
    uint64_t sh_addralign;
    uint64_t sh_entsize;
};

struct Elf64_Sym {
    uint32_t st_name;
    unsigned char st_info;
    unsigned char st_other;
    uint16_t st_shndx;
    uint64_t st_value;
    uint64_t st_size;
};
";

/// A 32-bit ELF. Every field that holds an address or an offset is four bytes
/// rather than eight, which moves everything after it.
const ELF32: &str = "\
struct Elf32_Ehdr {
    unsigned char e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint32_t e_entry;
    uint32_t e_phoff;
    uint32_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
};

struct Elf32_Phdr {
    uint32_t p_type;
    uint32_t p_offset;
    uint32_t p_vaddr;
    uint32_t p_paddr;
    uint32_t p_filesz;
    uint32_t p_memsz;
    uint32_t p_flags;
    uint32_t p_align;
};

struct Elf32_Shdr {
    uint32_t sh_name;
    uint32_t sh_type;
    uint32_t sh_flags;
    uint32_t sh_addr;
    uint32_t sh_offset;
    uint32_t sh_size;
    uint32_t sh_link;
    uint32_t sh_info;
    uint32_t sh_addralign;
    uint32_t sh_entsize;
};
";

/// A PE, as `winnt.h` declares it. The 64-bit optional header; a 32-bit one
/// differs from it after `BaseOfCode`.
const PE: &str = "\
struct IMAGE_DOS_HEADER {
    uint16_t e_magic;
    uint16_t e_cblp;
    uint16_t e_cp;
    uint16_t e_crlc;
    uint16_t e_cparhdr;
    uint16_t e_minalloc;
    uint16_t e_maxalloc;
    uint16_t e_ss;
    uint16_t e_sp;
    uint16_t e_csum;
    uint16_t e_ip;
    uint16_t e_cs;
    uint16_t e_lfarlc;
    uint16_t e_ovno;
    uint16_t e_res[4];
    uint16_t e_oemid;
    uint16_t e_oeminfo;
    uint16_t e_res2[10];
    uint32_t e_lfanew;
};

struct IMAGE_FILE_HEADER {
    uint16_t Machine;
    uint16_t NumberOfSections;
    uint32_t TimeDateStamp;
    uint32_t PointerToSymbolTable;
    uint32_t NumberOfSymbols;
    uint16_t SizeOfOptionalHeader;
    uint16_t Characteristics;
};

struct IMAGE_DATA_DIRECTORY {
    uint32_t VirtualAddress;
    uint32_t Size;
};

struct IMAGE_OPTIONAL_HEADER64 {
    uint16_t Magic;
    unsigned char MajorLinkerVersion;
    unsigned char MinorLinkerVersion;
    uint32_t SizeOfCode;
    uint32_t SizeOfInitializedData;
    uint32_t SizeOfUninitializedData;
    uint32_t AddressOfEntryPoint;
    uint32_t BaseOfCode;
    uint64_t ImageBase;
    uint32_t SectionAlignment;
    uint32_t FileAlignment;
    uint16_t MajorOperatingSystemVersion;
    uint16_t MinorOperatingSystemVersion;
    uint16_t MajorImageVersion;
    uint16_t MinorImageVersion;
    uint16_t MajorSubsystemVersion;
    uint16_t MinorSubsystemVersion;
    uint32_t Win32VersionValue;
    uint32_t SizeOfImage;
    uint32_t SizeOfHeaders;
    uint32_t CheckSum;
    uint16_t Subsystem;
    uint16_t DllCharacteristics;
    uint64_t SizeOfStackReserve;
    uint64_t SizeOfStackCommit;
    uint64_t SizeOfHeapReserve;
    uint64_t SizeOfHeapCommit;
    uint32_t LoaderFlags;
    uint32_t NumberOfRvaAndSizes;
    struct IMAGE_DATA_DIRECTORY DataDirectory[16];
};

struct IMAGE_SECTION_HEADER {
    char Name[8];
    uint32_t VirtualSize;
    uint32_t VirtualAddress;
    uint32_t SizeOfRawData;
    uint32_t PointerToRawData;
    uint32_t PointerToRelocations;
    uint32_t PointerToLinenumbers;
    uint16_t NumberOfRelocations;
    uint16_t NumberOfLinenumbers;
    uint32_t Characteristics;
};

struct IMAGE_IMPORT_DESCRIPTOR {
    uint32_t OriginalFirstThunk;
    uint32_t TimeDateStamp;
    uint32_t ForwarderChain;
    uint32_t Name;
    uint32_t FirstThunk;
};
";

/// A 64-bit Mach-O, as `<mach-o/loader.h>` declares it.
const MACH_O: &str = "\
struct mach_header_64 {
    uint32_t magic;
    uint32_t cputype;
    uint32_t cpusubtype;
    uint32_t filetype;
    uint32_t ncmds;
    uint32_t sizeofcmds;
    uint32_t flags;
    uint32_t reserved;
};

struct load_command {
    uint32_t cmd;
    uint32_t cmdsize;
};

struct segment_command_64 {
    uint32_t cmd;
    uint32_t cmdsize;
    char segname[16];
    uint64_t vmaddr;
    uint64_t vmsize;
    uint64_t fileoff;
    uint64_t filesize;
    int32_t maxprot;
    int32_t initprot;
    uint32_t nsects;
    uint32_t flags;
};

struct section_64 {
    char sectname[16];
    char segname[16];
    uint64_t addr;
    uint64_t size;
    uint32_t offset;
    uint32_t align;
    uint32_t reloff;
    uint32_t nreloc;
    uint32_t flags;
    uint32_t reserved1;
    uint32_t reserved2;
    uint32_t reserved3;
};
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Endianness,
        types::{Model, Registry, Type, parse},
    };

    fn registry(source: &str) -> Registry {
        let mut registry = Registry::new(Model {
            pointer: 8,
            long: 8,
            endianness: Endianness::Little,
        });
        for definition in parse::definitions(source).expect("the declarations read") {
            registry.define(definition);
        }
        registry
    }

    fn size_of(registry: &Registry, name: &str) -> u64 {
        registry
            .layout(&Type::Named(name.to_owned()))
            .unwrap_or_else(|error| panic!("{name} lays out: {error}"))
            .size
    }

    /// Every one of these is a documented, fixed number that the format's own
    /// readers depend on. A declaration that lays out to anything else has a
    /// field of the wrong width in it, and would put every field after that
    /// one at the wrong offset.
    #[test]
    fn every_format_lays_out_to_the_size_its_own_documentation_states() {
        let elf64 = registry(ELF64);
        assert_eq!(size_of(&elf64, "Elf64_Ehdr"), 64);
        assert_eq!(size_of(&elf64, "Elf64_Phdr"), 56);
        assert_eq!(size_of(&elf64, "Elf64_Shdr"), 64);
        assert_eq!(size_of(&elf64, "Elf64_Sym"), 24);

        let elf32 = registry(ELF32);
        assert_eq!(size_of(&elf32, "Elf32_Ehdr"), 52);
        assert_eq!(size_of(&elf32, "Elf32_Phdr"), 32);
        assert_eq!(size_of(&elf32, "Elf32_Shdr"), 40);

        let pe = registry(PE);
        assert_eq!(size_of(&pe, "IMAGE_DOS_HEADER"), 64);
        assert_eq!(size_of(&pe, "IMAGE_FILE_HEADER"), 20);
        assert_eq!(size_of(&pe, "IMAGE_OPTIONAL_HEADER64"), 240);
        assert_eq!(size_of(&pe, "IMAGE_SECTION_HEADER"), 40);
        assert_eq!(size_of(&pe, "IMAGE_IMPORT_DESCRIPTOR"), 20);

        let mach_o = registry(MACH_O);
        assert_eq!(size_of(&mach_o, "mach_header_64"), 32);
        assert_eq!(size_of(&mach_o, "load_command"), 8);
        assert_eq!(size_of(&mach_o, "segment_command_64"), 72);
        assert_eq!(size_of(&mach_o, "section_64"), 80);
    }

    /// The header each format is offered with is one the declarations define.
    #[test]
    fn the_header_a_format_starts_with_is_one_of_its_own_declarations() {
        for format in [
            BinaryFormat::Elf {
                bits: 64,
                endianness: Endianness::Little,
            },
            BinaryFormat::Elf {
                bits: 32,
                endianness: Endianness::Little,
            },
            BinaryFormat::Pe,
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little,
            },
        ] {
            let source = of(format).expect("declarations for this format");
            let name = header_of(format).expect("a header for this format");
            assert!(
                registry(source).get(name).is_some(),
                "{name} is defined by what {format:?} is offered with"
            );
        }
    }

    /// A format Desdec could not read is offered nothing, rather than the
    /// declarations of a format it is not.
    #[test]
    fn a_format_that_was_not_recognised_is_offered_nothing() {
        assert!(of(BinaryFormat::Unknown).is_none());
        assert!(header_of(BinaryFormat::Unknown).is_none());
    }
}
