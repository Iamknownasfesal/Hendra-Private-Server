; What a behaviour file is made of, in the four colours that matter: the words that structure it,
; the names of things it does, the names of its arguments, and the values.
;
; No node is captured twice: a name in quotes is a value where it is passed to something and the
; enemy itself where it heads a block, and the two want different colours rather than an argument
; about which pattern wins.

(argument value: (string) @string)
(escape) @string.escape
(number) @number
(duration) @number
(comment) @comment

["{" "}" "(" ")"] @punctuation.bracket
["," ":"] @punctuation.delimiter
"->" @operator

["enemy" "state" "on" "loot"] @keyword

(behaviour call: (call name: (identifier) @function))
(loot_entry call: (call name: (identifier) @function))

; A transition's condition reads as part of the `on` that introduces it.
(transition condition: (call name: (identifier) @function.special))

(argument name: (identifier) @property)
(argument value: (identifier) @constant)

; The enemy a file defines and the states it moves between: the three things that are declared
; rather than called, and the three worth following.
(enemy name: (string) @type)
(state name: (identifier) @constructor)
(transition target: (identifier) @constructor)
