//! What the file calls its addresses.
//!
//! An expression is far easier to write — and far easier to read back a week
//! later — about `main` than about `0x000000000000
//! 1a40`, and a reader who has just found a function in the table should be
//! able to write its name into a breakpoint condition without going back for
//! the number. This is the table that makes that possible.
//!
//! Every name in it comes from the file or from the reader: the symbol table,
//! the functions worked out from the code, and whatever the reader has named
//! an address themselves. Nothing is invented, and an address with no name
//! stays a number.

use std::collections::HashMap;

use desdec_core::Analysis;

use crate::{annotations::Annotations, ui::functions::Function};

/// Names and the addresses they stand for.
#[derive(Clone, Debug, Default)]
pub struct Table {
    by_name: HashMap<String, u64>,
}

impl Table {
    /// Builds the table for one analysed binary.
    ///
    /// The symbol table first and the discovered functions after, so a name
    /// the file states is never overwritten by one this tool made up: a
    /// function found from a call is named `sub_1a40`, and if the file had a
    /// real name for that address the real one is what a reader means.
    #[must_use]
    pub fn of(analysis: &Analysis, functions: &[Function]) -> Self {
        let mut by_name = HashMap::new();
        for symbol in &analysis.symbols {
            if let Some(address) = symbol.address {
                by_name.entry(symbol.name.clone()).or_insert(address);
            }
        }
        for function in functions {
            by_name
                .entry(function.name.clone())
                .or_insert(function.start);
        }
        Self { by_name }
    }

    /// The address a name stands for, and `None` for a name the file never
    /// gave to anything.
    #[must_use]
    pub fn address_of(&self, name: &str) -> Option<u64> {
        self.by_name.get(name).copied()
    }
}

/// What to call an address, and how far into that thing it is.
///
/// The other direction of [`Table`], and the one a cross-reference needs: a
/// list of the twenty places that name an address is twenty hexadecimal
/// numbers, and a reader has to walk to each of them to find out that
/// nineteen were the same function. `main+0x2c` is the whole answer, on the
/// row, before anyone goes anywhere.
///
/// Four answers, and the strongest is given: what the reader named the address
/// themselves, then the imported name the loader is to write into it, then the
/// function or symbol whose extent covers it, and failing all of those the
/// section it is in. Nothing is invented — an address in nothing named stays a
/// number, and the caller shows the number.
#[must_use]
pub fn describe(
    address: u64,
    analysis: &Analysis,
    functions: &[Function],
    annotations: &Annotations,
) -> Option<String> {
    if let Some(label) = annotations.label(address) {
        return Some(label.to_owned());
    }
    // An import slot holds nothing until the loader fills it in, so the file
    // itself is the only thing that can say whose address belongs there.
    if let Some(import) = analysis.import_at(address) {
        return Some(import.to_owned());
    }
    // The functions are in address order and their extents do not overlap, so
    // the one that can hold this address is the last one starting at or before
    // it — and it holds it only if it really reaches that far.
    let past = functions.partition_point(|function| function.start <= address);
    if let Some(function) = functions[..past]
        .last()
        .filter(|function| address < function.end)
    {
        return Some(with_offset(&function.name, address - function.start));
    }
    let section = analysis.section_at(address)?;
    Some(with_offset(
        &section.name,
        address.saturating_sub(section.virtual_address),
    ))
}

/// `main` at its first byte, and `main+0x2c` anywhere else inside it.
fn with_offset(name: &str, offset: u64) -> String {
    if offset == 0 {
        name.to_owned()
    } else {
        format!("{name}+{offset:#x}")
    }
}

#[cfg(test)]
mod tests {
    use super::{Table, describe};
    use desdec_core::Symbol;

    /// A real analysis, with its symbol table replaced by the one a test is
    /// about: `Analysis` is built by reading a file, not by naming fields.
    fn table_over(symbols: Vec<Symbol>) -> Table {
        let mut analysis = crate::testing::samples()
            .into_iter()
            .next()
            .expect("a fixture")
            .analysis;
        analysis.symbols = symbols;
        Table::of(&analysis, &[])
    }

    fn named(name: &str, address: u64) -> Symbol {
        Symbol {
            name: name.to_owned(),
            address: Some(address),
            size: 0,
            imported: false,
        }
    }

    /// The answer a cross-reference row needs: not a number, but the name of
    /// the thing the number is inside, and how far in.
    #[test]
    fn an_address_is_named_by_the_place_it_falls_in_and_how_far_into_it() {
        let sample = crate::testing::samples()
            .into_iter()
            .next()
            .expect("a fixture");
        let functions = crate::ui::functions::all(&sample.analysis);
        let function = functions.first().expect("the fixture names a function");
        let mut annotations = crate::annotations::Annotations::default();

        assert_eq!(
            describe(function.start, &sample.analysis, &functions, &annotations),
            Some(function.name.clone())
        );
        assert_eq!(
            describe(
                function.start + 4,
                &sample.analysis,
                &functions,
                &annotations
            ),
            Some(format!("{}+0x4", function.name)),
            "inside a function is that function, and says how far inside"
        );
        // An address in nothing at all stays a number, and the caller shows it.
        assert_eq!(
            describe(0xffff_ffff_0000, &sample.analysis, &functions, &annotations),
            None
        );

        // What the reader called it themselves outranks everything: they are
        // the one who has to recognise the row.
        annotations.set(
            function.start,
            crate::annotations::Annotation {
                label: "the parser".to_owned(),
                ..crate::annotations::Annotation::default()
            },
        );
        assert_eq!(
            describe(function.start, &sample.analysis, &functions, &annotations),
            Some("the parser".to_owned())
        );
    }

    #[test]
    fn a_name_the_file_states_stands_for_its_address() {
        let table = table_over(vec![named("main", 0x1a40)]);
        assert_eq!(table.address_of("main"), Some(0x1a40));
        assert_eq!(table.address_of("nowhere"), None);
    }

    /// A symbol with no address names nothing, and must not be offered as
    /// standing for zero.
    #[test]
    fn a_symbol_without_an_address_is_not_in_the_table() {
        let table = table_over(vec![Symbol {
            name: String::from("puts"),
            address: None,
            size: 0,
            imported: true,
        }]);
        assert_eq!(table.address_of("puts"), None);
    }

    /// The same name twice keeps the first address: a file that repeats a
    /// symbol must not have the meaning of a name depend on table order.
    #[test]
    fn a_name_stated_twice_keeps_the_first_address_it_was_given() {
        let table = table_over(vec![named("start", 0x1000), named("start", 0x2000)]);
        assert_eq!(table.address_of("start"), Some(0x1000));
    }
}
