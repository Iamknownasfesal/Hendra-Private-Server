//! Prints what a behaviour file compiled to, for checking arguments against the content.
use hendra_behavior::compile::compile;
use hendra_behavior::parse::parse;

fn main() {
    let source = std::env::args().nth(1).expect("a .beh file");
    let wanted = std::env::args().nth(2);
    let text = std::fs::read_to_string(&source).expect("readable");
    let parsed = parse(&text).expect("parses");
    let (programs, _) = compile(&parsed);

    for program in &programs.programs {
        if wanted.as_deref().is_some_and(|w| !program.name.contains(w)) {
            continue;
        }
        println!("enemy {:?}  names={:?}", program.name, program.names);
        for state in &program.states {
            for b in &state.behaviours {
                println!("  [{}] {b:?}", state.name);
            }
            for t in &state.transitions {
                println!("  [{}] on {:?}", state.name, t.condition);
            }
        }
    }
}
