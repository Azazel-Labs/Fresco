# Fresco TextMate grammar

`fresco.tmLanguage.json` is a standalone syntax-highlighting grammar for `.fr`
files, with the scope `source.fresco`. It uses standard TextMate scopes so editors
can apply their existing themes. It covers declarations, keywords, types, calls,
comments, strings, color literals, numeric units, host identifiers, and operators.

The grammar follows `crates/fresco/src/lexer.rs` and the authored examples.
It provides lexical highlighting, not compiler validation or semantic resolution.
Triple-quoted strings are raw; ordinary strings recognize Fresco's supported
escapes. Block comments do not nest, matching the lexer.

An editor extension can register this file for the language ID `fresco`, extension
`.fr`, and scope `source.fresco`. This directory does not install an extension or
enable highlighting on GitHub. GitHub support requires a separate Linguist
contribution registering the language and grammar.

Run the grammar tests with the actual TextMate/Oniguruma tokenizer:

```sh
cd syntaxes
npm ci
npm test
```

The tests cover token boundaries and multiline states, check compiler keyword and
unit coverage, and tokenize all repository examples. The Node dependencies are
only needed for tests; consumers need the JSON grammar alone.
