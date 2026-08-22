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

use crate::ui::functions::Function;

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

#[cfg(test)]
mod tests {
    use super::Table;
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
