//! What a tool does to the world: a closed class per tool, separate from
//! [`Tool::read_only`](super::Tool::read_only) and from [`Capabilities`].
//!
//! The axis is `docs/ROLLOUT-DESIGN.md` §3.2's, and it has two consumers:
//! a rollout's dry world decides from it alone what a call may do, and
//! code mode (`docs/CODE-MODE-DESIGN.md` §2) admits a tool into a program by
//! it. Neither consumer is built yet. This module is metadata, and nothing
//! may read it to loosen a guard: the interlock, the jail, approval and taint
//! all decide exactly as they did before it existed.
//!
//! **`read_only()` cannot stand in for it.** `read_only()` means "skips the
//! approval gate", not "has no effect": `message_send`, `image_generate`,
//! `persona_propose`, `hud_propose` and `ask_user` are all `read_only()`, and
//! none of them is a read.
//!
//! **Earned in code, never claimed by a server.** A built-in declares its
//! [`Effect`] in its `Tool` impl. The first-party servers (`mecha-mail`,
//! `mecha-docs` and the graph's pinned revision) are declared here, in
//! [`first_party`], by the server program and the tool's own name. A
//! third-party MCP tool has no class until the operator's own `[[mcp]]`
//! entry names one (`effects`), and the server's annotations can only move a
//! declared class toward caution: `destructiveHint` and `openWorldHint` reach
//! [`Capabilities`], which the resolver reads, and `readOnlyHint` is read by
//! nothing here. This is the rule [`Egress::Blind`] already follows.

use super::{Capabilities, Egress, Tool};
use serde::{Deserialize, Serialize};

/// What a tool's author says it does, before routing and capabilities are
/// applied. [`resolve`] turns it into an [`EffectClass`].
///
/// There is no `Staged` here, because staging is the operator's routing
/// (`[outbox] tools`), never a tool's property. There is no undeclared variant
/// either: a tool that declares nothing returns `None` from
/// [`Tool::effect`](super::Tool::effect).
///
/// **Ordered by caution**, least first, so combining two declarations is
/// `max`. The operator's word and mecha's first-party table are combined that
/// way ([`server_tool_effect`]): either may make a tool more cautious, and
/// neither can make it less.
///
/// Also the type `[[mcp]] effects` parses into. Strict on purpose: config is
/// not an append-only store, so an unknown spelling is a load error naming
/// the valid ones, never a silent default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Reads, and changes nothing anywhere.
    Read,
    /// Reads, and records that it read (the graph's `query_log` and
    /// `retrieval_touch`).
    ReadThatRecords,
    /// Writes only inside the run's own jailed workspace. Only a built-in can
    /// honestly say this: an MCP server writes where it was spawned, not in a
    /// run's workspace, so `Config::validate` refuses it in `[[mcp]] effects`.
    Workspace,
    /// Writes to one of mecha's own stores, or to the owner: a mailbox, a
    /// proposal queue, a question for a person.
    StoreWrite,
    /// Draws a picture or a clip.
    Render,
    /// Changes something outside mecha that cannot be taken back.
    Irreversible,
}

impl Effect {
    /// The name config and `--json` use.
    pub fn as_str(&self) -> &'static str {
        EffectClass::from(*self).as_str()
    }
}

/// The resolved class: one per tool, from [`resolve`]. A closed set written
/// to `mecha tools --json`, so it is a wire format: [`EffectClass::from_wire`]
/// reads an unknown name as [`Undeclared`](EffectClass::Undeclared), the
/// cautious end, rather than failing the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectClass {
    Read,
    ReadThatRecords,
    /// Routed through the operator's outbox: the call is drafted, never run.
    Staged,
    Workspace,
    StoreWrite,
    Render,
    Irreversible,
    /// Nobody has classified it. Unknown is never safe: a rollout halts at it
    /// and a program never sees it.
    Undeclared,
}

impl EffectClass {
    /// Every class, in §3.2's table order.
    pub const ALL: [EffectClass; 8] = [
        EffectClass::Read,
        EffectClass::ReadThatRecords,
        EffectClass::Staged,
        EffectClass::Workspace,
        EffectClass::StoreWrite,
        EffectClass::Render,
        EffectClass::Irreversible,
        EffectClass::Undeclared,
    ];

    /// The name `mecha tools --json` prints. Add a variant and this match is
    /// what forces you to name it.
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectClass::Read => "read",
            EffectClass::ReadThatRecords => "read_that_records",
            EffectClass::Staged => "staged",
            EffectClass::Workspace => "workspace",
            EffectClass::StoreWrite => "store_write",
            EffectClass::Render => "render",
            EffectClass::Irreversible => "irreversible",
            EffectClass::Undeclared => "undeclared",
        }
    }

    /// A name read back off the wire. One this build does not know, from a
    /// newer build or a typo, is [`Undeclared`](EffectClass::Undeclared):
    /// it degrades to the class that admits nothing, never to an error and
    /// never to a read.
    pub fn from_wire(name: &str) -> EffectClass {
        EffectClass::ALL
            .into_iter()
            .find(|c| c.as_str() == name)
            .unwrap_or(EffectClass::Undeclared)
    }
}

impl From<Effect> for EffectClass {
    fn from(e: Effect) -> Self {
        match e {
            Effect::Read => EffectClass::Read,
            Effect::ReadThatRecords => EffectClass::ReadThatRecords,
            Effect::Workspace => EffectClass::Workspace,
            Effect::StoreWrite => EffectClass::StoreWrite,
            Effect::Render => EffectClass::Render,
            Effect::Irreversible => EffectClass::Irreversible,
        }
    }
}

impl Serialize for EffectClass {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EffectClass {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Any string at all is accepted; see `from_wire`.
        let name = String::deserialize(d)?;
        Ok(EffectClass::from_wire(&name))
    }
}

/// One class per tool, `docs/ROLLOUT-DESIGN.md` §3.2's order, first match
/// wins. The rules overlap as written (`mail_send` is routed *and* `Chosen`;
/// `fs_write` is `Workspace` *and* `destructive`), so the order is what makes
/// the table a partition.
///
/// 1. Routed by the operator's outbox: `Staged`. A routed call never runs.
/// 2. Declared by nobody: `Undeclared`. A server's hints declare nothing.
/// 3. `Egress::Chosen`: `Irreversible`. This is where an unconfined `shell`
///    and `http_fetch` land.
/// 4. Declared `Workspace`: `Workspace`, before the destructive row, or the
///    scratch-workspace half of rollouts is unreachable (`fs_write`,
///    `fs_edit` and confined `shell` all declare `destructive`).
/// 5. `destructive`, or declared `Irreversible`: `Irreversible`. The second
///    half is not in §3.2's numbered list, which reaches `Irreversible` only
///    through the two capability rows; without it `calendar_hold`,
///    `docs_create` and the graph's writes, none of them `destructive` or
///    `Chosen`, would fall through to a read.
/// 6. to 9. What the tool declared: `Render`, `StoreWrite`,
///    `ReadThatRecords`, `Read`.
///
/// `routed` is the caller's: whether this tool's name is in the operator's
/// `[outbox] tools` for the run being asked about.
pub fn resolve(declared: Option<Effect>, caps: Capabilities, routed: bool) -> EffectClass {
    if routed {
        return EffectClass::Staged;
    }
    let Some(declared) = declared else {
        return EffectClass::Undeclared;
    };
    if caps.egress == Egress::Chosen {
        return EffectClass::Irreversible;
    }
    if declared == Effect::Workspace {
        return EffectClass::Workspace;
    }
    if caps.destructive || declared == Effect::Irreversible {
        return EffectClass::Irreversible;
    }
    match declared {
        Effect::Render => EffectClass::Render,
        Effect::StoreWrite => EffectClass::StoreWrite,
        Effect::ReadThatRecords => EffectClass::ReadThatRecords,
        Effect::Read => EffectClass::Read,
        // Both returned above; named rather than `_` so a new variant has to
        // be placed in the order by hand.
        Effect::Workspace | Effect::Irreversible => unreachable!("handled above"),
    }
}

/// [`resolve`] over a tool as it is registered: its declaration and its
/// capabilities, which already carry the sandbox's narrowing and any
/// operator `[mcp.capabilities]` widening.
pub fn class_of(tool: &dyn Tool, routed: bool) -> EffectClass {
    resolve(tool.effect(), tool.capabilities(), routed)
}

/// The MCP servers whose tools mecha classifies in its own code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstParty {
    /// `mecha-mail`: mail and calendar behind one surface.
    Mail,
    /// `mecha-docs`: Google Docs, Sheets and Slides.
    Docs,
    /// `mecha-graph-mcp`, at the revision `graph_mcp::REV` pins.
    Graph,
}

impl FirstParty {
    /// Which first-party server an `[[mcp]]` entry runs, by the file name of
    /// its `command`: the one match `feature::server_feature` already makes
    /// for the feature gate, so the two can never disagree about which
    /// program is which. `factory-publish` belongs to a feature too, and is
    /// not first-party here: its source is not in this repository.
    ///
    /// The program's name is a claim the *config* makes, not the server, so
    /// the caller must only ask this of an entry the operator wrote
    /// (`McpServerConfig::from_operator`). A project file naming
    /// `./tools/mecha-mail` would otherwise buy a third-party binary mecha's
    /// own classes.
    pub fn of(server: &crate::config::McpServerConfig) -> Option<FirstParty> {
        use crate::feature::Feature;
        match crate::feature::server_feature(server)? {
            Feature::Mail => Some(FirstParty::Mail),
            Feature::Docs => Some(FirstParty::Docs),
            Feature::Graph => Some(FirstParty::Graph),
            _ => None,
        }
    }
}

/// mecha's own table for its first-party servers, by the server's own
/// (unprefixed) tool name. `None` for a name not listed: a tool a newer
/// server added is undeclared until it is listed here, the same as a
/// third-party one.
///
/// A send is declared `Irreversible` because it is one: routed through the
/// outbox it resolves `Staged` (rule 1), and unrouted its `openWorldHint`
/// makes it `Chosen`, which resolves `Irreversible` whatever is declared.
pub fn first_party(server: FirstParty, tool: &str) -> Option<Effect> {
    use Effect::*;
    Some(match (server, tool) {
        // mecha-mail (`mecha-mail/src/unified.rs`, `tool_definitions`).
        (
            FirstParty::Mail,
            "mail_search"
            | "mail_recent"
            | "mail_get_thread"
            | "calendar_list"
            | "calendar_list_events"
            | "calendar_freebusy",
        ) => Read,
        (
            FirstParty::Mail,
            "mail_send"
            | "mail_reply"
            | "mail_triage"
            | "calendar_create_event"
            | "calendar_hold"
            | "calendar_update_event"
            | "calendar_delete_event",
        ) => Irreversible,
        // mecha-docs (`mecha-mail/src/google/docs_server.rs`).
        (FirstParty::Docs, "docs_list" | "docs_read" | "sheets_read" | "slides_read") => Read,
        (
            FirstParty::Docs,
            "docs_create" | "docs_append" | "docs_replace" | "docs_trash" | "sheets_create"
            | "sheets_write" | "slides_create",
        ) => Irreversible,
        // The graph at `graph_mcp::REV`. Every read is `ReadThatRecords`. At
        // the pin only `kg_search` writes `query_log` and `retrieval_touch`
        // (through `router::query_lens`), but the build on the PATH may be the
        // owner's own checkout rather than the pin (`sidecar.rs`), and the
        // cost of the wider class is nil: a program admits both, and a
        // rollout needs ruling W1's unrecorded mode before it touches the
        // graph at all.
        (
            FirstParty::Graph,
            "kg_search" | "kg_entity" | "kg_related" | "kg_timeline" | "kg_notes" | "kg_verify"
            | "kg_pending" | "kg_shadow_queue" | "kg_task_list",
        ) => ReadThatRecords,
        (FirstParty::Graph, "kg_upsert" | "kg_verdict" | "kg_task_create" | "kg_task_update") => {
            Irreversible
        }
        _ => return None,
    })
}

/// An MCP tool's declaration: mecha's first-party table and the operator's
/// `[[mcp]] effects` entry, the more cautious of the two when both speak.
/// The server's own annotations are not an input. They reach the class only
/// through [`Capabilities`], in [`resolve`], where they can only add caution.
pub fn server_tool_effect(first_party: Option<Effect>, operator: Option<Effect>) -> Option<Effect> {
    first_party.max(operator)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(egress: Egress, destructive: bool) -> Capabilities {
        Capabilities {
            egress,
            destructive,
            ..Capabilities::default()
        }
    }

    const DECLARED: [Option<Effect>; 7] = [
        None,
        Some(Effect::Read),
        Some(Effect::ReadThatRecords),
        Some(Effect::Workspace),
        Some(Effect::StoreWrite),
        Some(Effect::Render),
        Some(Effect::Irreversible),
    ];

    /// The resolver, restated as §3.2's numbered rules with the one addition
    /// `resolve` documents, each as an independent predicate. Every input
    /// must match at least one rule (the table covers everything), and the
    /// first one that matches must be what `resolve` returns.
    #[test]
    fn the_table_is_a_partition_in_the_designed_order() {
        type Rule = (EffectClass, fn(Option<Effect>, Capabilities, bool) -> bool);
        let rules: [Rule; 9] = [
            (EffectClass::Staged, |_, _, r| r),
            (EffectClass::Undeclared, |d, _, _| d.is_none()),
            (EffectClass::Irreversible, |_, c, _| {
                c.egress == Egress::Chosen
            }),
            (EffectClass::Workspace, |d, _, _| {
                d == Some(Effect::Workspace)
            }),
            (EffectClass::Irreversible, |d, c, _| {
                c.destructive || d == Some(Effect::Irreversible)
            }),
            (EffectClass::Render, |d, _, _| d == Some(Effect::Render)),
            (EffectClass::StoreWrite, |d, _, _| {
                d == Some(Effect::StoreWrite)
            }),
            (EffectClass::ReadThatRecords, |d, _, _| {
                d == Some(Effect::ReadThatRecords)
            }),
            (EffectClass::Read, |d, _, _| d == Some(Effect::Read)),
        ];
        let mut reached = std::collections::HashSet::new();
        for declared in DECLARED {
            for egress in [Egress::None, Egress::Blind, Egress::Chosen] {
                for destructive in [false, true] {
                    for routed in [false, true] {
                        let c = caps(egress, destructive);
                        let first = rules
                            .iter()
                            .find(|(_, matches)| matches(declared, c, routed))
                            .map(|(class, _)| *class)
                            .expect("every input is covered by some rule");
                        assert_eq!(
                            resolve(declared, c, routed),
                            first,
                            "{declared:?} {egress:?} destructive={destructive} routed={routed}"
                        );
                        reached.insert(first);
                    }
                }
            }
        }
        // Every class is reachable: no row of the table is dead.
        for class in EffectClass::ALL {
            assert!(reached.contains(&class), "{class:?} is unreachable");
        }
    }

    /// The overlaps §3.2 names, each settled by the order and not by luck.
    #[test]
    fn the_overlaps_resolve_the_way_the_design_orders_them() {
        // `mail_send` is routed and `Chosen`: routing wins.
        assert_eq!(
            resolve(
                Some(Effect::Irreversible),
                caps(Egress::Chosen, false),
                true
            ),
            EffectClass::Staged
        );
        // `calendar_delete_event` is routed and destructive: routing wins.
        assert_eq!(
            resolve(Some(Effect::Irreversible), caps(Egress::Chosen, true), true),
            EffectClass::Staged
        );
        // A routed third-party tool nobody classified still never runs.
        assert_eq!(
            resolve(None, caps(Egress::Chosen, false), true),
            EffectClass::Staged
        );
        // Undeclared before `Chosen`: a hint is not a declaration.
        assert_eq!(
            resolve(None, caps(Egress::Chosen, true), false),
            EffectClass::Undeclared
        );
        // `fs_write`: `Workspace` before destructive.
        assert_eq!(
            resolve(Some(Effect::Workspace), caps(Egress::None, true), false),
            EffectClass::Workspace
        );
        // An unconfined `shell`: `Chosen` before `Workspace`.
        assert_eq!(
            resolve(Some(Effect::Workspace), caps(Egress::Chosen, true), false),
            EffectClass::Irreversible
        );
        // `image_library_reference` declares a store write and `destructive`:
        // the numbered order makes it `Irreversible` (see the PR's note).
        assert_eq!(
            resolve(Some(Effect::StoreWrite), caps(Egress::None, true), false),
            EffectClass::Irreversible
        );
        // `web_search` is a read whose query leaves blind: still a read.
        assert_eq!(
            resolve(Some(Effect::Read), caps(Egress::Blind, false), false),
            EffectClass::Read
        );
        // `calendar_hold`: not destructive, not `Chosen`, declared.
        assert_eq!(
            resolve(Some(Effect::Irreversible), caps(Egress::None, false), false),
            EffectClass::Irreversible
        );
    }

    #[test]
    fn an_unknown_name_on_the_wire_degrades_to_undeclared() {
        for class in EffectClass::ALL {
            let wire = serde_json::to_value(class).unwrap();
            assert_eq!(wire, serde_json::json!(class.as_str()));
            let back: EffectClass = serde_json::from_value(wire).unwrap();
            assert_eq!(back, class, "round trip");
        }
        let newer: EffectClass = serde_json::from_value(serde_json::json!("quantum")).unwrap();
        assert_eq!(newer, EffectClass::Undeclared);
        // Inside a record, the unknown name costs the field, not the record.
        #[derive(Deserialize)]
        struct Row {
            name: String,
            effect: EffectClass,
        }
        let row: Row =
            serde_json::from_str(r#"{"name": "kg_dream", "effect": "speculative"}"#).unwrap();
        assert_eq!(row.name, "kg_dream");
        assert_eq!(row.effect, EffectClass::Undeclared);
    }

    #[test]
    fn a_declaration_in_config_is_strict() {
        #[derive(Deserialize)]
        struct Entry {
            #[allow(dead_code)]
            effect: Effect,
        }
        assert!(toml::from_str::<Entry>("effect = \"read_that_records\"").is_ok());
        // Config is not a wire: a typo is an error naming the valid ones.
        let typo = toml::from_str::<Entry>("effect = \"raed\"")
            .err()
            .unwrap()
            .to_string();
        assert!(typo.contains("read_that_records"), "{typo}");
        // Staging is the outbox's to decide, and undeclared is the absence of
        // an entry: neither is a spelling a declaration can use.
        for not_a_declaration in ["staged", "undeclared"] {
            assert!(
                toml::from_str::<Entry>(&format!("effect = \"{not_a_declaration}\"")).is_err(),
                "{not_a_declaration}"
            );
        }
    }

    /// The operator and mecha's table combine toward caution only.
    #[test]
    fn an_override_moves_only_toward_caution() {
        assert_eq!(
            server_tool_effect(Some(Effect::Read), Some(Effect::Irreversible)),
            Some(Effect::Irreversible)
        );
        assert_eq!(
            server_tool_effect(Some(Effect::Irreversible), Some(Effect::Read)),
            Some(Effect::Irreversible),
            "an operator cannot promote a first-party write to a read"
        );
        assert_eq!(
            server_tool_effect(None, Some(Effect::Render)),
            Some(Effect::Render)
        );
        assert_eq!(server_tool_effect(None, None), None);
    }

    #[test]
    fn the_first_party_tables_name_the_cases_the_designs_name() {
        let mail = |t| first_party(FirstParty::Mail, t);
        for read in ["mail_search", "mail_get_thread", "calendar_list_events"] {
            assert_eq!(mail(read), Some(Effect::Read), "{read}");
        }
        for write in ["calendar_hold", "mail_triage", "mail_send"] {
            assert_eq!(mail(write), Some(Effect::Irreversible), "{write}");
        }
        assert_eq!(
            first_party(FirstParty::Docs, "docs_read"),
            Some(Effect::Read)
        );
        for write in ["docs_create", "docs_trash"] {
            assert_eq!(
                first_party(FirstParty::Docs, write),
                Some(Effect::Irreversible),
                "{write}"
            );
        }
        assert_eq!(
            first_party(FirstParty::Graph, "kg_search"),
            Some(Effect::ReadThatRecords)
        );
        assert_eq!(
            first_party(FirstParty::Graph, "kg_upsert"),
            Some(Effect::Irreversible)
        );
        // A tool the table does not list is undeclared, first-party or not.
        assert_eq!(first_party(FirstParty::Graph, "fetch"), None);
        assert_eq!(first_party(FirstParty::Mail, "docs_read"), None);
    }
}
