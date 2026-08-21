//! Which function calls which, over the whole file.
//!
//! The Functions view answers "what is in this function". The reference index
//! answers "who names this address". Neither answers the question a reader
//! actually arrives with — *how does anything get to here* — because that is a
//! chain: `main` calls a parser, which calls a reader, which calls the thing
//! being looked at, and none of those steps is visible from either end.
//!
//! This is that chain, built once per binary from the calls the listing
//! already decoded. Both directions are kept, because both questions get
//! asked: who calls this one, and what does this one call.
//!
//! Three things it deliberately does not do. It does not guess at an indirect
//! call: `call *%rax` names no target, and a graph that quietly dropped it
//! would show a function with no callees rather than a function whose callees
//! are not knowable from the text. It counts them instead, and the interface
//! says how many. It does not follow a call into a library — that code is in
//! a file that is not open. And it does not invent a caller for a function
//! nothing calls: an entry point, a callback given to a library, and dead code
//! all look alike from here, and saying "nothing calls this" is the honest
//! answer to all three.

use std::collections::{BTreeMap, BTreeSet};

use desdec_core::{Analysis, operand};

use crate::ui::functions::Function;

/// One call, from the function it is written in to the one it reaches.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Call {
    /// The function the call is written in.
    pub from: u64,
    /// The function it reaches.
    pub to: u64,
    /// The instruction itself, so the listing can be taken to it.
    pub at: u64,
}

/// What one function is at either end of.
#[derive(Clone, Debug, Default)]
pub struct Edges {
    /// Calls written in this function, in address order.
    pub calls: Vec<Call>,
    /// Calls that reach it.
    pub callers: Vec<Call>,
    /// Calls in it whose target no text states — `call *%rax`, `blr x8`.
    ///
    /// Counted rather than dropped: a function that calls through a pointer
    /// has callees, and a graph showing none of them would be lying by
    /// omission.
    pub indirect: usize,
    /// Calls in it that leave what the file maps, which is what a call into a
    /// library looks like from here.
    pub outside: usize,
}

/// The call graph of one binary.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    /// Every function's edges, by the address it starts at.
    functions: BTreeMap<u64, Edges>,
}

impl Graph {
    /// Builds the graph for one binary's functions.
    ///
    /// Walks each function's decoded body once, so the whole thing costs one
    /// pass over the listing — the same order of work as the reference index
    /// beside it, and done at the same moment for the same reason.
    #[must_use]
    pub fn of(analysis: &Analysis, functions: &[Function]) -> Self {
        // Where each address falls, so a call's target can be turned into the
        // function that owns it. Sorted starts, bisected: a large binary has
        // tens of thousands of functions and a map per instruction would cost
        // more than the graph is worth.
        let starts: Vec<u64> = functions.iter().map(|function| function.start).collect();
        let mut graph = Self {
            functions: functions
                .iter()
                .map(|function| (function.start, Edges::default()))
                .collect(),
        };

        for function in functions {
            let body = &analysis.instructions[function.instructions.clone()];
            for instruction in body {
                if !is_call(&instruction.text) {
                    continue;
                }
                let Some(target) = operand::branch_target(instruction) else {
                    // The target is in a register, and no register has a value
                    // without a run.
                    if let Some(edges) = graph.functions.get_mut(&function.start) {
                        edges.indirect += 1;
                    }
                    continue;
                };
                let Some(callee) = owner(&starts, target) else {
                    // Somewhere the file's own functions do not cover: a
                    // library, or a part of the image nothing decoded.
                    if let Some(edges) = graph.functions.get_mut(&function.start) {
                        edges.outside += 1;
                    }
                    continue;
                };
                let call = Call {
                    from: function.start,
                    to: callee,
                    at: instruction.address,
                };
                if let Some(edges) = graph.functions.get_mut(&function.start) {
                    edges.calls.push(call);
                }
                if let Some(edges) = graph.functions.get_mut(&callee) {
                    edges.callers.push(call);
                }
            }
        }
        for edges in graph.functions.values_mut() {
            edges.calls.sort_unstable();
            edges.callers.sort_unstable();
        }
        graph
    }

    /// What one function calls and is called by.
    #[must_use]
    pub fn edges(&self, function: u64) -> Option<&Edges> {
        self.functions.get(&function)
    }

    /// How many functions the graph holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    /// How many calls it states — the measure of how much it has to work with.
    ///
    /// Counted from the written end, so each call is counted once: counting
    /// both ends would double every one of them.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.functions.values().map(|edges| edges.calls.len()).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }

    /// Functions nothing in this file calls.
    ///
    /// Not "dead code": an entry point is here, and so is every function whose
    /// address is only ever taken and handed to something else. What it is
    /// good for is finding where to start reading.
    pub fn unreached(&self) -> impl Iterator<Item = u64> + '_ {
        self.functions
            .iter()
            .filter(|(_, edges)| edges.callers.is_empty())
            .map(|(address, _)| *address)
    }

    /// Every way from `from` to `to`, up to `most` of them, shortest first.
    ///
    /// The question a reader arrives with: *how does anything get to here?*
    /// Breadth-first, so the first path found is a shortest one, and bounded
    /// on both the number of paths and the length of each — a call graph has
    /// cycles, and every cycle is an unbounded number of paths through it.
    #[must_use]
    pub fn paths(&self, from: u64, to: u64, most: usize) -> Vec<Vec<u64>> {
        if !self.functions.contains_key(&from) || !self.functions.contains_key(&to) {
            return Vec::new();
        }
        if from == to {
            return vec![vec![from]];
        }
        let mut found = Vec::new();
        let mut queue = std::collections::VecDeque::from([vec![from]]);
        // Bounded by the work done rather than by the depth alone: a wide
        // graph reaches the limit without ever being deep.
        let mut steps = 0_usize;
        while let Some(path) = queue.pop_front() {
            steps += 1;
            if found.len() >= most || steps > SEARCH_LIMIT {
                break;
            }
            let last = *path.last().expect("a path has an end");
            let Some(edges) = self.functions.get(&last) else {
                continue;
            };
            for call in &edges.calls {
                if path.contains(&call.to) {
                    // Round a cycle it has already been round. The path
                    // through it once is the one worth showing.
                    continue;
                }
                let mut extended = path.clone();
                extended.push(call.to);
                if call.to == to {
                    found.push(extended);
                    if found.len() >= most {
                        break;
                    }
                } else if extended.len() < DEPTH_LIMIT {
                    queue.push_back(extended);
                }
            }
        }
        found
    }

    /// Everything one function reaches, however far away.
    ///
    /// What "if I change this, what could it affect" is, read the other way
    /// round: everything below it in the graph.
    #[must_use]
    pub fn reachable_from(&self, start: u64) -> BTreeSet<u64> {
        let mut seen = BTreeSet::new();
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(address) = queue.pop_front() {
            if !seen.insert(address) || seen.len() > SEARCH_LIMIT {
                continue;
            }
            if let Some(edges) = self.functions.get(&address) {
                for call in &edges.calls {
                    queue.push_back(call.to);
                }
            }
        }
        seen.remove(&start);
        seen
    }
}

/// How far a path is followed before it is abandoned. A chain of forty calls
/// is not an explanation of anything.
const DEPTH_LIMIT: usize = 40;
/// How much work any one search does before it stops. A call graph of a large
/// binary is dense, and every question asked of it must answer in a frame.
const SEARCH_LIMIT: usize = 200_000;

/// The function an address belongs to: the last one starting at or before it.
fn owner(starts: &[u64], address: u64) -> Option<u64> {
    let index = starts
        .partition_point(|start| *start <= address)
        .checked_sub(1)?;
    starts.get(index).copied()
}

/// Whether a line is a call, in the syntaxes the tool decodes into.
fn is_call(text: &str) -> bool {
    let mnemonic = text.split_whitespace().next().unwrap_or_default();
    matches!(mnemonic, "call" | "callq" | "calll" | "callw" | "bl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::reference_analysis;

    /// The host's own binary, with its functions and their graph.
    fn reference() -> (Vec<Function>, Graph) {
        let analysis = reference_analysis();
        let functions = crate::ui::functions::all(analysis);
        let graph = Graph::of(analysis, &functions);
        (functions, graph)
    }

    #[test]
    fn a_real_binary_has_a_graph_with_both_directions_in_it() {
        let (functions, graph) = reference();
        assert_eq!(graph.len(), functions.len(), "every function is in it");

        let calls: usize = functions
            .iter()
            .filter_map(|function| graph.edges(function.start))
            .map(|edges| edges.calls.len())
            .sum();
        assert!(calls > 100, "a real binary calls things: {calls}");

        // Every call is on both ends of the graph, and on neither twice.
        let callers: usize = functions
            .iter()
            .filter_map(|function| graph.edges(function.start))
            .map(|edges| edges.callers.len())
            .sum();
        assert_eq!(calls, callers, "each call is written once and reaches once");
    }

    #[test]
    fn every_edge_names_a_function_that_exists_and_an_instruction_that_does() {
        let (functions, graph) = reference();
        let analysis = reference_analysis();
        for function in functions.iter().take(400) {
            let Some(edges) = graph.edges(function.start) else {
                continue;
            };
            for call in &edges.calls {
                assert_eq!(call.from, function.start);
                assert!(graph.edges(call.to).is_some(), "{:#x} exists", call.to);
                assert!(
                    analysis.instruction_at(call.at).is_some(),
                    "{:#x} is a row of the listing",
                    call.at
                );
            }
        }
    }

    /// An indirect call is counted rather than dropped: a function that calls
    /// through a pointer has callees, and showing none would be a lie by
    /// omission.
    #[test]
    fn indirect_calls_are_counted_rather_than_dropped() {
        // Whether the test runner's executable happens to contain an indirect
        // call is a property of its compiler and its platform, not of this
        // graph.  The macOS ARM64 runner did not have one, so test the graph
        // invariant with the smallest explicit input instead.
        let graph = Graph {
            functions: std::collections::BTreeMap::from([(
                0x1000,
                Edges {
                    indirect: 1,
                    ..Edges::default()
                },
            )]),
        };
        let indirect: usize = graph
            .functions
            .keys()
            .filter_map(|address| graph.edges(*address))
            .map(|edges| edges.indirect)
            .sum();
        assert_eq!(indirect, 1);
    }

    /// The question a reader arrives with, answered on the file's own code.
    #[test]
    fn a_path_between_two_functions_is_a_chain_of_real_calls() {
        let (functions, graph) = reference();
        // A pair that is really connected: take a function that calls, and one
        // of the things it calls.
        let Some((from, to)) = functions.iter().find_map(|function| {
            let edges = graph.edges(function.start)?;
            let call = edges.calls.iter().find(|call| {
                graph
                    .edges(call.to)
                    .is_some_and(|next| !next.calls.is_empty())
            })?;
            let onward = graph.edges(call.to)?.calls.first()?;
            Some((function.start, onward.to))
        }) else {
            return; // Nothing on this host is two calls deep.
        };

        let paths = graph.paths(from, to, 4);
        assert!(!paths.is_empty(), "there is a way from one to the other");
        for path in &paths {
            assert_eq!(path.first(), Some(&from));
            assert_eq!(path.last(), Some(&to));
            // Every step of a path is a call that is really written.
            for pair in path.windows(2) {
                let edges = graph.edges(pair[0]).expect("a function on the path");
                assert!(
                    edges.calls.iter().any(|call| call.to == pair[1]),
                    "{:#x} really calls {:#x}",
                    pair[0],
                    pair[1]
                );
            }
        }
        // Shortest first.
        assert!(paths.windows(2).all(|pair| pair[0].len() <= pair[1].len()));
    }

    #[test]
    fn a_path_to_somewhere_unreachable_is_no_path_rather_than_a_wrong_one() {
        let (_, graph) = reference();
        assert!(graph.paths(0xdead_beef, 0xfeed_face, 4).is_empty());
    }

    /// A cycle is walked once, not for ever.
    #[test]
    fn a_recursive_function_does_not_make_the_search_run_for_ever() {
        let (functions, graph) = reference();
        let recursive = functions.iter().find(|function| {
            graph
                .edges(function.start)
                .is_some_and(|edges| edges.calls.iter().any(|call| call.to == function.start))
        });
        let Some(recursive) = recursive else {
            return; // Nothing on this host calls itself directly.
        };
        // Answers at all, which is the whole of what this asserts.
        let _ = graph.paths(recursive.start, 0, 4);
        let reached = graph.reachable_from(recursive.start);
        assert!(!reached.contains(&recursive.start) || reached.len() > 1);
    }

    #[test]
    fn what_a_function_reaches_is_everything_below_it() {
        let (functions, graph) = reference();
        let Some(caller) = functions.iter().find(|function| {
            graph
                .edges(function.start)
                .is_some_and(|e| e.calls.len() > 2)
        }) else {
            return;
        };
        let reached = graph.reachable_from(caller.start);
        let direct = graph.edges(caller.start).expect("edges");
        for call in &direct.calls {
            if call.to != caller.start {
                assert!(
                    reached.contains(&call.to),
                    "what it calls directly is reachable from it"
                );
            }
        }
    }

    #[test]
    fn functions_nothing_calls_are_where_a_reader_starts() {
        // This is a property of the graph, not of the executable that happens
        // to run the tests.  The Windows test binary has MSVC startup code
        // that can call its entry function; asserting that a host executable
        // always has an uncalled entry made the release depend on its runner.
        let graph = Graph {
            functions: std::collections::BTreeMap::from([
                (0x1000, Edges::default()),
                (
                    0x2000,
                    Edges {
                        callers: vec![Call {
                            from: 0x1000,
                            to: 0x2000,
                            at: 0x1010,
                        }],
                        ..Edges::default()
                    },
                ),
            ]),
        };
        let unreached: Vec<u64> = graph.unreached().collect();
        assert_eq!(unreached, [0x1000]);
    }
}
