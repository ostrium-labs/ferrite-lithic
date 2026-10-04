//! Stable names, and the three-state allocation that makes them stable.
//!
//! # The contract
//!
//! Two runs of the same program emit byte-identical Verilog. That is a
//! *requirement*, not a nice property, because the only way to test an emitter is
//! to diff its output.
//!
//! Hardcaml has to defend that with work: node identity is a `uid` from a
//! **process-global mutable counter** (`kernel/signal__type.ml:966`), so
//! reproducibility depends on the program running the same way twice, and there is
//! a `normalize_uids` pass to rewrite ids into DFS pre-order — **off by default**,
//! with its own test commented out as "brittle to changes in the test
//! environment" (`test/lib/test_uid_normalization.ml:56-59`).
//!
//! We get it structurally instead. Node ids *are* arena indices, and arena indices
//! are construction order by definition, so walking the graph in id order is
//! already a canonical order with no pass to normalise and nothing global to thread
//! through. What is left to decide is which name each node gets, and that is this
//! module.
//!
//! # Three states, in this order
//!
//! 1. **Ports**, in port-list order — `src/circuit.ml:83-104, 164-211`. A port's
//!    identifier is its ABI, so it is allocated before anything can take the name,
//!    and under [`PortNames::Wire`] it is the name on the wire verbatim.
//! 2. **Explicitly named signals**, in id order. User names win over derived ones.
//! 3. **Unnamed signals**, in id order — deliberately *not* depth-first, so names
//!    land in roughly the order the logic does rather than in the order a recursive
//!    walk happens to reach them.
//!
//! # Collisions, and how they are broken
//!
//! Derived names are `signal_<node_kind>` (`src/rtl_name.ml:182-210`), so two
//! adds collide by construction. A collision appends `_<n>` and retries
//! (`src/mangler.ml:32-39`), starting at 1. Matching is **case-insensitive**
//! (`src/rtl_name.ml:19-30`): Verilog identifiers are case-sensitive, so this is
//! stricter than it needs to be, and stricter is the point — a design that
//! elaborates on one machine and not another because two signals differed only in
//! case is not a difference worth debugging.
//!
//! Verilog reserved words get a trailing `_`, and anything that is not a legal
//! identifier is rewritten character by character, so `9 lives` becomes `_9_lives`
//! rather than being refused. A sanitised name can collide with a real one; the
//! suffix rule is what keeps the result unambiguous either way.

use std::collections::{BTreeMap, BTreeSet};

use ferrite_lithic_ir::{Node, NodeId};

use crate::error::Error;
use crate::module::{Direction, Module, PortNames};

/// Verilog-2001 reserved words, plus the SystemVerilog-2005 core that every tool
/// in this project accepts.
///
/// The comparison is case-insensitive, which is stricter than Verilog: `INPUT` is
/// a legal identifier in a case-sensitive language. Reserving it costs a design
/// nothing (a signal named `input` becomes `input_`, which it can reach anyway by
/// being renamed) and saves the emitter from being the thing that breaks when a
/// tool grows a keyword.
const RESERVED: &[&str] = &[
    "alias",
    "always",
    "assume",
    "always_comb",
    "always_ff",
    "always_latch",
    "and",
    "assign",
    "automatic",
    "before",
    "begin",
    "bind",
    "bins",
    "binsof",
    "bit",
    "break",
    "buf",
    "byte",
    "bufif0",
    "bufif1",
    "case",
    "casex",
    "casez",
    "cell",
    "chandle",
    "checker",
    "class",
    "clocking",
    "cmos",
    "config",
    "const",
    "constraint",
    "context",
    "continue",
    "cover",
    "covergroup",
    "coverpoint",
    "cross",
    "deassign",
    "default",
    "defparam",
    "design",
    "disable",
    "dist",
    "do",
    "edge",
    "else",
    "end",
    "endcase",
    "endchecker",
    "endclass",
    "endclocking",
    "endconfig",
    "endfunction",
    "endgenerate",
    "endgroup",
    "endinterface",
    "endmodule",
    "endpackage",
    "endprimitive",
    "endprogram",
    "endproperty",
    "endspecify",
    "endsequence",
    "endtable",
    "endtask",
    "enum",
    "event",
    "expect",
    "export",
    "extends",
    "extern",
    "final",
    "first_match",
    "for",
    "force",
    "foreach",
    "forever",
    "fork",
    "forkjoin",
    "function",
    "generate",
    "genvar",
    "global",
    "highz0",
    "highz1",
    "if",
    "iff",
    "ifnone",
    "ignore_bins",
    "illegal_bins",
    "implements",
    "implies",
    "import",
    "incdir",
    "include",
    "initial",
    "inout",
    "input",
    "inside",
    "instance",
    "int",
    "integer",
    "interconnect",
    "interface",
    "intersect",
    "join",
    "join_any",
    "join_none",
    "large",
    "let",
    "liblist",
    "library",
    "local",
    "localparam",
    "logic",
    "longint",
    "macromodule",
    "matches",
    "medium",
    "modport",
    "module",
    "nand",
    "negedge",
    "nettype",
    "new",
    "nexttime",
    "nmos",
    "nor",
    "noshowcancelled",
    "not",
    "notif0",
    "notif1",
    "null",
    "or",
    "output",
    "package",
    "packed",
    "parameter",
    "pmos",
    "posedge",
    "primitive",
    "priority",
    "program",
    "property",
    "protected",
    "pull0",
    "pull1",
    "pulldown",
    "pullup",
    "pulsestyle_ondetect",
    "pulsestyle_onevent",
    "pure",
    "rand",
    "randc",
    "randcase",
    "randsequence",
    "rcmos",
    "real",
    "realtime",
    "ref",
    "reg",
    "reject_on",
    "release",
    "repeat",
    "restrict",
    "return",
    "rnmos",
    "rpmos",
    "rtran",
    "rtranif0",
    "rtranif1",
    "s_always",
    "s_eventually",
    "s_nexttime",
    "s_until",
    "s_until_with",
    "scalared",
    "sequence",
    "shortint",
    "shortreal",
    "showcancelled",
    "signed",
    "small",
    "soft",
    "solve",
    "specify",
    "specparam",
    "static",
    "string",
    "strong",
    "strong0",
    "strong1",
    "struct",
    "super",
    "supply0",
    "supply1",
    "sync_accept_on",
    "sync_reject_on",
    "table",
    "tagged",
    "task",
    "this",
    "throughout",
    "throw",
    "time",
    "timeprecision",
    "timeunit",
    "tran",
    "tranif0",
    "tranif1",
    "tri",
    "tri0",
    "tri1",
    "triand",
    "trior",
    "trireg",
    "type",
    "typedef",
    "union",
    "unique",
    "unique0",
    "unsigned",
    "until",
    "until_with",
    "untyped",
    "use",
    "uwire",
    "var",
    "vectored",
    "virtual",
    "void",
    "wait",
    "wait_order",
    "wand",
    "weak",
    "weak0",
    "weak1",
    "while",
    "wildcard",
    "wire",
    "with",
    "within",
    "wor",
    "xnor",
    "xor",
];

/// Whether a name collides with a reserved word, ignoring case.
fn is_reserved(name: &str) -> bool {
    RESERVED.iter().any(|word| word.eq_ignore_ascii_case(name))
}

/// The emitter's legalisation with no pool, for a name that must stay exactly as
/// written.
///
/// # Why this is a public entry point
///
/// A port list's Verilog names have to be rewritten the same way every signal
/// name is, or a module declares a port that cannot be connected. That rewrite
/// cannot live in `ferrite-lithic`, where the shape is declared, because the
/// legalisation rules and the reserved words they avoid belong to the emitter.
/// So `#[rtlmangle]` on a port list calls this.
///
/// No pool, deliberately. Port names are per-module and a module's port list is
/// what an instantiation site binds to, so a name that gets a numeric suffix
/// because some *other* name in the same scope claimed its legal form would be a
/// port that moved. Two ports that both legalise to the same thing are a genuine
/// collision in the user's interface, and the right answer is for that to be an
/// error rather than for the emitter to silently renumber one of them.
#[must_use]
pub fn mangle_port_name(name: &str) -> String {
    legalise(name)
}

/// A legal, non-reserved identifier built from `name`.
///
/// Every character outside `[A-Za-z0-9_$]` becomes `_`, a leading digit gets a `_`
/// in front, and a reserved word gets a `_` appended. Nothing is refused: a name
/// that cannot be spelled in Verilog still has to be *representable* in the output,
/// and a rewritten name is visible in a diff while a refusal is only visible in an
/// error message.
pub(crate) fn legalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 1);
    for (index, ch) in name.chars().enumerate() {
        if index == 0 && ch.is_ascii_digit() {
            out.push('_');
        }
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if is_reserved(&out) {
        out.push('_');
    }
    out
}

/// Hands out identifiers, and remembers which are gone.
///
/// Keys are lowercased, which is where the case-insensitive matching of the
/// reference comes from: Verilog would allow `acc` and `ACC` side by side, and this
/// emitter will not.
#[derive(Debug, Default)]
struct Mangler {
    taken: BTreeSet<String>,
}

impl Mangler {
    /// Claim a name without using it.
    fn reserve(&mut self, name: &str) {
        self.taken.insert(name.to_ascii_lowercase());
    }

    /// The first free identifier derived from `base`.
    fn take(&mut self, base: &str) -> String {
        let mut candidate = base.to_string();
        let mut suffix = 1u32;
        while self.taken.contains(&candidate.to_ascii_lowercase()) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        self.reserve(&candidate);
        candidate
    }
}

/// Every identifier a module needs, assigned before a single character is written.
#[derive(Debug, Default)]
pub(crate) struct NameMap {
    names: BTreeMap<NodeId, String>,
    labels: BTreeMap<NodeId, String>,
}

impl NameMap {
    /// Build the map for `module` over the nodes in `reachable`.
    ///
    /// `reachable` must be in id order; the third state allocates from it in that
    /// order, and the whole determinism argument rests on it.
    pub(crate) fn build(module: &Module, reachable: &[NodeId]) -> Result<Self, Error> {
        let circuit = module.circuit();
        let mut map = Self::default();
        let mut mangler = Mangler::default();

        // A node in both port lists is an inout, which this emitter does not model.
        for id in module.inputs() {
            if module.outputs().contains(id) {
                return Err(Error::PortIsBothDirections { id: *id });
            }
        }

        // State 0: positional port names are reserved before anything else can
        // claim them, or a signal called `i0` would break an instance binding.
        if module.port_names() == PortNames::Positional {
            for index in 0..module.inputs().len() {
                mangler.reserve(&format!("i{index}"));
            }
            for index in 0..module.outputs().len() {
                mangler.reserve(&format!("o{index}"));
            }
        }

        // State 1: ports, in port-list order.
        for (ids, direction) in [
            (module.inputs(), Direction::Input),
            (module.outputs(), Direction::Output),
        ] {
            for (index, id) in ids.iter().enumerate() {
                let name = match module.port_names() {
                    PortNames::Positional => {
                        let base = match direction {
                            Direction::Input => format!("i{index}"),
                            Direction::Output => format!("o{index}"),
                        };
                        // Reserved above, so this cannot fail; a suffix here would
                        // silently break every instantiation site.
                        base
                    }
                    PortNames::Wire => {
                        let raw = circuit
                            .name(*id)
                            .ok_or(Error::UnnamedPort { direction, id: *id })?;
                        if raw.is_empty() {
                            return Err(Error::EmptySignalName { id: *id });
                        }
                        // Deliberately *not* run through `take`: a port keeps its
                        // name, and a second port wanting it gets the suffix rather
                        // than the first one silently renamed.
                        let legal = legalise(raw);
                        let mut candidate = legal.clone();
                        let mut suffix = 1u32;
                        while mangler.taken.contains(&candidate.to_ascii_lowercase()) {
                            candidate = format!("{legal}_{suffix}");
                            suffix += 1;
                        }
                        mangler.reserve(&candidate);
                        candidate
                    }
                };
                map.names.insert(*id, name);
            }
        }

        // State 2: explicit names, in id order. `Circuit::names` is keyed by id, so
        // this is construction order and not alphabetical order.
        for (id, raw) in circuit.names() {
            if map.names.contains_key(&id) {
                continue;
            }
            if raw.is_empty() {
                return Err(Error::EmptySignalName { id });
            }
            let name = mangler.take(&legalise(raw));
            map.names.insert(id, name);
        }

        // State 3: unnamed signals, in graph order.
        for id in reachable {
            if map.names.contains_key(id) {
                continue;
            }
            let node = circuit.get(*id)?;
            // A constant is rendered inline wherever it is used, so it never needs
            // an identifier. Allocating one would put a name in the output that
            // nothing reads.
            if matches!(node, Node::Constant { .. }) {
                continue;
            }
            let kind = node.kind();
            let name = mangler.take(&format!("signal_{kind}"));
            map.names.insert(*id, name);
            // An instantiation also needs a *label*, which lives in the same
            // namespace: a label colliding with a signal is a Verilog error.
            if matches!(node, Node::Instance { .. }) {
                let label = mangler.take(&format!("u_{kind}"));
                map.labels.insert(*id, label);
            }
        }

        Ok(map)
    }

    /// The identifier for a node.
    pub(crate) fn name(&self, id: NodeId) -> &str {
        self.names.get(&id).map_or("<unnamed>", String::as_str)
    }

    /// The instance label for a node, if it is an instantiation.
    pub(crate) fn label(&self, id: NodeId) -> &str {
        self.labels.get(&id).map_or("<unlabelled>", String::as_str)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{Mangler, is_reserved, legalise};

    #[test]
    fn reserved_words_are_matched_ignoring_case() {
        assert!(is_reserved("input"));
        assert!(is_reserved("INPUT"));
        assert!(is_reserved("EndModule"));
        assert!(!is_reserved("input_"));
        assert!(!is_reserved("acc"));
    }

    #[test]
    fn illegal_characters_become_underscores() {
        assert_eq!(legalise("a-b"), "a_b");
        assert_eq!(legalise("a.b"), "a_b");
        assert_eq!(legalise("has space"), "has_space");
        assert_eq!(legalise("naïve"), "na_ve");
        assert_eq!(legalise("$ok"), "$ok");
        assert_eq!(legalise("_ok"), "_ok");
    }

    #[test]
    fn a_leading_digit_gets_a_prefix() {
        assert_eq!(legalise("9lives"), "_9lives");
        assert_eq!(legalise("1"), "_1");
    }

    #[test]
    fn a_reserved_word_gets_a_suffix() {
        assert_eq!(legalise("input"), "input_");
        assert_eq!(legalise("wire"), "wire_");
        assert_eq!(legalise("module"), "module_");
    }

    #[test]
    fn a_name_that_is_only_illegal_characters_still_has_an_identifier() {
        assert_eq!(legalise("---"), "___");
    }

    #[test]
    fn collisions_are_broken_with_a_numeric_suffix() {
        let mut mangler = Mangler::default();
        assert_eq!(mangler.take("signal_Add"), "signal_Add");
        assert_eq!(mangler.take("signal_Add"), "signal_Add_1");
        assert_eq!(mangler.take("signal_Add"), "signal_Add_2");
        assert_eq!(mangler.take("signal_BitAnd"), "signal_BitAnd");
    }

    #[test]
    fn collision_matching_ignores_case() {
        let mut mangler = Mangler::default();
        assert_eq!(mangler.take("acc"), "acc");
        assert_eq!(mangler.take("ACC"), "ACC_1");
        assert_eq!(mangler.take("Acc"), "Acc_2");
    }

    #[test]
    fn the_common_systemverilog_keywords_are_all_reserved() {
        // A keyword list is a list, and a list has one way to go wrong: someone adds
        // a language feature and forgets the keyword. `byte` was missing for the
        // whole of A1 and A2, and a design with a port called `byte` emitted
        // `input wire [7:0] byte`, which is a syntax error in every tool that
        // matters. Nothing caught it because nothing was ever named `byte`.
        //
        // So this is the check that the list is plausible, not exhaustive: a keyword
        // chosen because it is likely to be a port name, or because it is easy to
        // reach for by accident.
        for word in [
            "always",
            "and",
            "assign",
            "automatic",
            "begin",
            "bit",
            "break",
            "buf",
            "byte",
            "case",
            "cell",
            "chandle",
            "class",
            "const",
            "constraint",
            "cover",
            "default",
            "defparam",
            "do",
            "else",
            "end",
            "endmodule",
            "enum",
            "event",
            "expect",
            "final",
            "for",
            "force",
            "forever",
            "fork",
            "function",
            "generate",
            "genvar",
            "if",
            "iff",
            "initial",
            "inout",
            "input",
            "inside",
            "int",
            "integer",
            "join",
            "large",
            "let",
            "liblist",
            "library",
            "local",
            "logic",
            "longint",
            "macromodule",
            "module",
            "nand",
            "negedge",
            "new",
            "nor",
            "not",
            "null",
            "output",
            "package",
            "parameter",
            "pmos",
            "posedge",
            "primitive",
            "priority",
            "program",
            "property",
            "pullup",
            "pulldown",
            "pure",
            "rand",
            "randc",
            "real",
            "realtime",
            "ref",
            "reg",
            "release",
            "repeat",
            "return",
            "rnmos",
            "rpmos",
            "scalared",
            "shortint",
            "shortreal",
            "signed",
            "small",
            "specify",
            "specparam",
            "static",
            "string",
            "struct",
            "super",
            "supply0",
            "supply1",
            "table",
            "tagged",
            "task",
            "this",
            "throw",
            "time",
            "timeprecision",
            "tran",
            "tri",
            "tri0",
            "tri1",
            "trireg",
            "type",
            "typedef",
            "union",
            "unsigned",
            "until",
            "var",
            "vectored",
            "virtual",
            "void",
            "wait",
            "wand",
            "weak",
            "while",
            "wildcard",
            "wire",
            "with",
            "within",
            "wor",
            "xnor",
            "xor",
        ] {
            assert!(is_reserved(word), "`{word}` is not in the reserved list");
        }
    }

    #[test]
    fn a_reserved_name_suffixed_by_the_caller_does_not_steal_the_escaped_one() {
        let mut mangler = Mangler::default();
        // `input` escapes to `input_`, which is also a legal user name.
        assert_eq!(mangler.take(&legalise("input")), "input_");
        assert_eq!(mangler.take("input_"), "input__1");
    }
}
