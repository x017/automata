//! End-to-end check of the language-server client against a real server.
//!
//! Runs when a Python language server is installed (or named by
//! `OBERON_TEST_LSP`, e.g. `/path/to/jedi-language-server`); otherwise it
//! prints a note and passes.

use std::time::{Duration, Instant};

use oberon::code::{CodeEditor, CodeEvent, Key, Mods, Pos};
use oberon::lsp::{LspAction, LspClient};

fn pump(
    lsp: &mut LspClient,
    ed: &mut CodeEditor,
    until: impl Fn(&CodeEditor) -> bool,
) -> Vec<LspAction> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut actions = Vec::new();
    while Instant::now() < deadline {
        lsp.sync(ed);
        actions.extend(lsp.poll(ed));
        if until(ed) || lsp.is_dead() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    actions
}

#[test]
fn completion_hover_definition_and_diagnostics() {
    let dir = std::env::temp_dir().join(format!("oberon-lsp-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("game.py");
    std::fs::write(
        &path,
        "class Player:\n    def jump_high(self) -> None:\n        \"\"\"Makes the player jump.\"\"\"\n\n\np = Player()\np.\n",
    )
    .unwrap();

    let command = std::env::var("OBERON_TEST_LSP").ok();
    let mut lsp = match LspClient::start(command.as_deref(), &dir, &[]) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    let mut ed = CodeEditor::new();
    ed.open(&path, Some(7)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !lsp.is_ready() && Instant::now() < deadline && !lsp.is_dead() {
        lsp.sync(&ed);
        lsp.poll(&mut ed);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(lsp.is_ready(), "{} did not initialize", lsp.name);

    // Completion after `p.`
    {
        let tab = ed.current_mut().unwrap();
        tab.buf.move_to(Pos::new(6, 2), false);
    }
    lsp.sync(&ed);
    lsp.request(&ed, &CodeEvent::Complete(path.clone(), Pos::new(6, 2)));
    pump(&mut lsp, &mut ed, |ed| ed.completion.is_some());
    let labels: Vec<String> = ed
        .completion
        .as_ref()
        .map(|c| c.items.iter().map(|i| i.label.clone()).collect())
        .unwrap_or_default();
    assert!(
        labels.iter().any(|l| l == "jump_high"),
        "{} completions: {labels:?}",
        lsp.name
    );
    assert!(ed.accept_completion());
    assert_eq!(ed.current().unwrap().buf.line(6), "p.jump_high");

    // Hover on the method.
    lsp.sync(&ed);
    lsp.request(&ed, &CodeEvent::Hover(path.clone(), Pos::new(6, 4)));
    pump(&mut lsp, &mut ed, |ed| ed.hover.is_some());
    let hover = ed.hover.clone().map(|h| h.1).unwrap_or_default();
    assert!(hover.contains("jump"), "{} hover: {hover:?}", lsp.name);

    // Go to definition.
    lsp.request(&ed, &CodeEvent::Definition(path.clone(), Pos::new(6, 4)));
    lsp.sync(&ed);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut goto = None;
    while goto.is_none() && Instant::now() < deadline {
        for a in lsp.poll(&mut ed) {
            if let LspAction::Goto(p, pos) = a {
                goto = Some((p, pos));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (gpath, gpos) = goto.expect("definition");
    assert_eq!(
        std::fs::canonicalize(gpath).unwrap(),
        std::fs::canonicalize(&path).unwrap()
    );
    assert_eq!(gpos.line, 1);

    // A syntax error produces a diagnostic.
    ed.key(Key::End, Mods::default());
    ed.text("(:");
    pump(&mut lsp, &mut ed, |ed| {
        !ed.current().unwrap().diagnostics.is_empty()
    });
    assert!(
        !ed.current().unwrap().diagnostics.is_empty(),
        "{} sent no diagnostics",
        lsp.name
    );

    lsp.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
