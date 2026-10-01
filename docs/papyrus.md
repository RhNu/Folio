# Skyrim Papyrus in Folio

This guide describes the Skyrim Papyrus behavior Folio currently recognizes and checks. A project selects `languages.papyrus.dialect = "skyrim"` and `build.target = "skyrim-se"`. Accepted syntax, target generation support, and available runtime APIs are separate requirements. `check` reports programs that cannot be analyzed or represented on the target.

## Sources and declarations

Source files use `.psc`. Scripts begin with `ScriptName` and can specify `Extends`, `Hidden`, and `Conditional`. Folio recognizes imports, variables, properties, named states, functions, events, parameter defaults, `Global`, `Native`, and project-declared custom flags.

Script and file names are compared without ASCII case sensitivity. Duplicate script names within a package are errors.

Function and event bodies support local variables, `Return`, assignments, `If`/`ElseIf`/`Else`, `While`, calls, and expressions. Expressions include literals, arithmetic, comparisons, logical operations, member access, array construction and indexing, and explicit `As` casts. Assignment `=` and equality `==` are distinct operations.

The syntax tree retains whitespace, semicolon line comments, `;/ ... /;` block comments, `{ ... }` documentation comments, line continuations, and error nodes. A damaged local construct does not discard the entire file. Constructs that cannot be safely analyzed or generated produce diagnostics.

## Types and conversions

Analysis handles built-in types, script references, and arrays in one model. It resolves inheritance, members, calls, argument bindings, and source locations. Unresolved names retain an error state that limits cascading diagnostics; they are not treated as valid `None` values. External declarations provide visible APIs, not the runtime implementations of a game or extension.

Skyrim conditions in `If`, `ElseIf`, and `While`, and operands of `&&`, `||`, and `!`, accept `Bool` and implicit Boolean conversions from `Int`, `Float`, `String`, script references, arrays, and `None`. Semantic analysis records those conversions explicitly, and lowering emits MIR `Cast` operations.

Boolean conversion does not relax comparison rules: numeric values, `Bool`, and `String` cannot be compared with `None` on that basis. Arrays can appear in conditions and in comparisons with `None`, but the two forms are not assumed to be equivalent.

`String` can be concatenated with `Int` or `Float` using `+`; `String +=` accepts those numeric types as well. Analysis selects the conversion, then lowering emits a string `Cast` followed by `StrCat`. Unsupported operations are rejected explicitly; floating-point remainder, for example, is not silently replaced with a different calculation.

## Calls and defaults

Named arguments bind to resolved parameters while preserving source evaluation order. By default, omitting a required argument without a default produces `semantic.argument-count`. With `fill-missing-arguments` enabled, the call site supplies the type's default value and reports `semantic.argument-defaulted`. Supplied arguments keep their original evaluation order.

PEX-derived APIs can have unknown parameter defaults and unknown callable kinds. Argument filling does not guess unknown defaults. See [Experimental PEX dependencies](architecture/project-model.md#experimental-pex-dependencies) for the supported uses and restrictions.

## Built-in operations

`GetState()` and `GotoState(String)` are compiler-provided instance methods. Folio generates state reads and transitions; a transition calls `OnEndState()`, updates the state, then calls `OnBeginState()`.

Array `Length`, `Find`, and `RFind` use target array operations. Engine native methods must be visible through project sources or external declarations. Built-in state and array operations do not make other engine scripts automatically visible.

Editors can retrieve resolved local types, definitions, and diagnostics even when other semantic errors exist. A build requires semantic, target, and MIR validation to succeed. See [Compiler pipeline](architecture/compiler.md) for phase boundaries and evaluation order, and [Projects and dependencies](architecture/project-model.md) for configuration and API visibility.
