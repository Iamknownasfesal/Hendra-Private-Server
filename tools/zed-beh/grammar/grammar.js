/// The behaviour language, as a tree-sitter grammar.
///
/// Mirrors `hendra_behavior::parse`: an enemy holds items, an item is a state, a transition or a
/// call, and a call followed by a block is a group such as `prioritize`. Nothing here knows which
/// behaviour names exist — that is the compiler's table, not the grammar's, so adding a primitive
/// never means regenerating a parser.

const sepBy = (separator, rule) =>
  optional(seq(rule, repeat(seq(separator, rule))));

module.exports = grammar({
  name: "beh",

  word: ($) => $.identifier,

  extras: ($) => [/[\s﻿]/, $.comment],

  rules: {
    source_file: ($) => repeat($.enemy),

    enemy: ($) =>
      seq("enemy", field("name", $.string), field("body", $.enemy_body)),

    enemy_body: ($) => seq("{", repeat(choice($.loot_table, $._item)), "}"),

    _item: ($) => choice($.state, $.transition, $.behaviour),

    state: ($) => seq("state", field("name", $.identifier), field("body", $.block)),

    block: ($) => seq("{", repeat($._item), "}"),

    // A call on its own does something; a call with a block, such as `prioritize`, contains the
    // items that follow it.
    behaviour: ($) => seq(field("call", $.call), optional(field("body", $.block))),

    transition: ($) =>
      seq(
        "on",
        field("condition", $.call),
        "->",
        field("target", $.identifier),
      ),

    call: ($) => seq(field("name", $.identifier), optional($.arguments)),

    arguments: ($) => seq("(", sepBy(",", $.argument), optional(","), ")"),

    argument: ($) =>
      seq(
        optional(seq(field("name", $.identifier), ":")),
        field("value", $._value),
      ),

    // A bare word is a value too: `true`, an effect name, a loot kind. It lexes as an identifier,
    // and what it means is decided by which argument it is, not by the grammar.
    _value: ($) => choice($.number, $.duration, $.string, $.identifier),

    // `loot` is only an enemy's own table, never something a state does.
    loot_table: ($) => seq("loot", field("body", $.loot_body)),

    loot_body: ($) => seq("{", repeat($.loot_entry), "}"),

    loot_entry: ($) =>
      seq(field("call", $.call), optional(field("body", $.loot_body))),

    identifier: ($) => /[A-Za-z_][A-Za-z0-9_]*/,

    // Milliseconds, however they were written. The suffix is part of the token so `12s` never
    // lexes as the number 12 followed by a word.
    duration: ($) => token(seq(optional("-"), /(\d+\.?\d*|\.\d+)/, choice("ms", "s"))),

    number: ($) => token(seq(optional("-"), /(\d+\.?\d*|\.\d+)/)),

    string: ($) =>
      seq(
        '"',
        repeat(choice($.escape, token.immediate(/[^"\\\n]+/))),
        '"',
      ),

    escape: ($) => token.immediate(seq("\\", /./)),

    comment: ($) => token(seq(choice("#", "//"), /[^\n]*/)),
  },
});
