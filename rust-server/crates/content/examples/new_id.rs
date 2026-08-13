//! Prints a fresh identity for new content.
//!
//! Content does not need one: without a `uuid` attribute the identity is derived from the name,
//! which is stable across runs and machines. Write one when the name might change and saved
//! inventories should follow the item rather than the label.
//!
//! ```text
//! $ cargo run -p hendra-content --example new_id -- "Wand of Dawn"
//! <Object uuid="0f8fad5b-d9cb-469f-a165-70867728950e" id="Wand of Dawn">
//! ```

fn main() {
    let name = std::env::args().nth(1);
    let uuid = hendra_content::identity::fresh();

    match name {
        Some(name) => println!(r#"<Object uuid="{uuid}" id="{name}">"#),
        None => println!("{uuid}"),
    }
}
