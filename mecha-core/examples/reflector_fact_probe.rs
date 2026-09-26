//! Does the *real* reflector answer D3's `fact` field? (row 2e-3)
//!
//! After 2e-3 every behaviour rule depends on the reflector's reply carrying
//! `"fact": true|false`, and, for a fact, spans copied word for word: no
//! answer is unknown and never mined, and a paraphrased span is ungrounded.
//! The unit tests drive a fixed-reply provider, which measures what we
//! *believe* the model emits and is structurally blind to what it does
//! (CLAUDE.md, "Testing without credentials"). This probe asks the
//! configured provider, the local model on this install, over fixture
//! interventions on the fictional cast, and reports per case what the
//! reflector answered and what `attribution::decide` made of it.
//!
//! It reads no store and writes nothing. It spends one model call per case,
//! so run it by hand, not from the nightly:
//!
//!   cargo run -q -p mecha-core --example reflector_fact_probe [provider]

use anyhow::{Context, Result};
use mecha_core::attribution::{decide, Answer, Class, Given};
use mecha_core::learning::{Intervention, Reflector, Trigger};
use mecha_core::message::{Block, Message};

/// One fixture: what the run read, what it said, what the owner said, and
/// the class a compliant reflector leads `decide` to.
struct Case {
    name: &'static str,
    trigger: Trigger,
    read: &'static str,
    said: &'static str,
    owner: &'static str,
    /// Hand the reflector the clean-evidence view, as `evidence_for` does
    /// when third-party content was in context.
    withheld: bool,
    expect: Class,
}

const CASES: &[Case] = &[
    Case {
        name: "fact, the graph had it wrong",
        trigger: Trigger::Followup,
        read: "Dana Whitfield: employer Northwind Labs (2024).",
        said: "Dana Whitfield works at Northwind Labs.",
        owner: "No, she moved to Lakeside Institute in the spring.",
        withheld: false,
        expect: Class::Data,
    },
    Case {
        name: "fact, the run had it right and misused it",
        trigger: Trigger::Followup,
        read: "Marek: office Room 214.",
        said: "Marek's office is Room 118.",
        owner: "That's the old one. Marek is in Room 214 now, not Room 118.",
        withheld: false,
        expect: Class::Behaviour,
    },
    // D3's first row, and the lookup's limit: a result that mentions the
    // old value only as history still holds it, so this reads as a data
    // error. The conservative direction: it is never mined.
    Case {
        name: "fact, the old value mentioned as history",
        trigger: Trigger::Followup,
        read: "Marek: office Room 214 (moved from Room 118 in August).",
        said: "Marek's office is Room 118.",
        owner: "That's the old one. Marek is in Room 214 now, not Room 118.",
        withheld: false,
        expect: Class::Data,
    },
    Case {
        name: "fact, nothing was read",
        trigger: Trigger::Steer,
        read: "No events found for Priya Nair this week.",
        said: "I'll book the review with Priya Nair for Friday.",
        owner: "Wrong day. The review with Priya is Thursday, not Friday.",
        withheld: false,
        expect: Class::Gap,
    },
    Case {
        name: "fact, context withheld (tainted conversation)",
        trigger: Trigger::Followup,
        read: "Rhea's deadline: the 14th.",
        said: "Rhea's deadline is the 14th.",
        owner: "No, Rhea's deadline moved to the 9th, not the 14th.",
        withheld: true,
        expect: Class::Data,
    },
    Case {
        name: "how, a denial",
        trigger: Trigger::Denial,
        read: "Draft ready for sam@example.edu.",
        said: "Sending the summary to Sam now.",
        owner: "don't send anything to Sam yet, I want to read it first",
        withheld: false,
        expect: Class::Behaviour,
    },
    Case {
        name: "how, a steer",
        trigger: Trigger::Steer,
        read: "config.toml\nREADME.md\nsrc/main.rs\nsrc/lib.rs",
        said: "Reading every file to find the port setting.",
        owner: "stop reading every file, just search for the port key",
        withheld: false,
        expect: Class::Behaviour,
    },
    Case {
        name: "how, a follow-up about length",
        trigger: Trigger::Followup,
        read: "(twelve paragraphs of meeting notes)",
        said: "Here is a detailed summary of the meeting in nine paragraphs.",
        owner: "Too long. Keep meeting summaries to three bullets.",
        withheld: false,
        expect: Class::Behaviour,
    },
];

fn transcript(c: &Case) -> Vec<Message> {
    vec![
        Message::user("Help me with this."),
        Message::assistant(vec![Block::ToolUse {
            id: "t1".into(),
            name: "kg_search".into(),
            input: serde_json::json!({"query": "context"}),
        }]),
        Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "t1".into(),
            content: c.read.into(),
            is_error: false,
        }]),
        Message::assistant(vec![Block::text(c.said)]),
        Message::user(c.owner),
    ]
}

#[tokio::main]
async fn main() -> Result<()> {
    let cwd = std::env::current_dir().context("cwd")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let wanted = std::env::args().nth(1);
    let (name, provider_cfg) = cfg.provider(wanted.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let reflector = Reflector::new(provider, provider_cfg.model.clone());
    eprintln!("probing {} ({name})", reflector.model());

    let (mut answered, mut placed, mut lessons) = (0usize, 0usize, 0usize);
    // `false` on every case would read as "answered" throughout, and is the
    // shape that mines everything as before: counted apart (review of #332).
    let mut no_fact = 0usize;
    for c in CASES {
        let messages = transcript(c);
        let full = Intervention {
            trigger: c.trigger,
            context: format!("kg_search {{\"query\":\"context\"}}\n{}", c.said),
            text: c.owner.into(),
            aftermath: String::new(),
            at: 4,
            tools_before: vec!["kg_search".into()],
            tools_after: Vec::new(),
        };
        let input = if c.withheld {
            full.user_evidence_only()
        } else {
            full.clone()
        };
        match reflector.reflect(&input).await? {
            None => println!("{:<48} no lesson (skip)", c.name),
            Some((_, answer)) => {
                lessons += 1;
                if answer != Answer::NotAnswered {
                    answered += 1;
                }
                if answer == Answer::NoFact {
                    no_fact += 1;
                }
                let a = decide(&answer, c.owner, &Given::before(&messages, 4));
                if a.class == c.expect {
                    placed += 1;
                }
                println!(
                    "{:<48} {:<10} expected {:<10} {:?} {:?}",
                    c.name,
                    a.class.as_str(),
                    c.expect.as_str(),
                    a.basis,
                    answer
                );
            }
        }
    }
    println!(
        "\n{lessons}/{} drew a lesson; of those, {answered} answered `fact` ({no_fact} of \
         them `false`) and {placed} landed in the expected class",
        CASES.len()
    );
    Ok(())
}
