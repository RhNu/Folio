# Papyrus language reference

This guide records the **Skyrim Papyrus baseline** used to develop Folio: source rules, static constraints, conversions, and behavior that affects compilation. It is not a common specification for every Papyrus game, or a claim that Folio has implemented every rule below.

The reference has three stable domains:

- This file: Skyrim source language and compiler compatibility constraints.
- [Dialects and target differences](papyrus/dialects.md): Fallout, Starfield, and other game-specific language differences.
- [Runtime semantics and built-in behavior](papyrus/runtime.md): dispatch, scheduling, persistence, failures, and special engine operations.

Folio currently selects `languages.papyrus.dialect = "skyrim"` and `build.target = "skyrim-se"`. Other documented dialects provide evidence for future development, not implemented parser or generation targets. Accepted syntax, target capabilities, and runtime API availability are separate requirements.

This reference excludes CK/SKSE API catalogs. Engine methods remain in scope when their special behavior affects language implementation. See [Folio behavior boundary](#folio-behavior-boundary) for existing implementation descriptions and the [conformance roadmap](planning/roadmap.md#papyrus-conformance) for identified implementation deviations and unresolved verification.

## Evidence and scope

The CK UESP wiki combines Bethesda reference material with community additions. A category or title does not establish that every paragraph is authoritative or independently tested. [Source coverage](#source-coverage) identifies the exact revisions reviewed on 2026-10-01.

| Classification | Development meaning |
| --- | --- |
| Language rule | Source grammar, static typing, binding, or described observable semantics |
| CK compiler restriction | Original compiler limitation, distinct from VM representability |
| CK compiler defect | Reported incorrect compilation; reproducing it is not automatically a requirement |
| Engine behavior | Dispatch, storage, target operations, or runtime failures |
| Community observation | Described behavior without a complete independent verification basis |
| Unresolved | Conflicting or insufficient evidence; no invented precise rule |

These classifications are editorial interpretations of evidence. Old compiler acceptance does not establish safe runtime semantics; compiler rejection does not necessarily establish a VM limitation.

The compiler architecture remains in [Compiler pipeline](architecture/compiler.md). Outstanding implementation and external verification belong in [Roadmap](planning/roadmap.md). Neither wiki review nor this documentation update proves engine compatibility.

## Lexical structure

### Files and identifiers

Skyrim source files use `.psc`. The first non-comment line is a ScriptName header. The declared name must match the filename without its extension. An optional Extends clause names one parent, followed by flags.

Identifiers begin with a letter or underscore; subsequent characters are letters, decimal digits, or underscores. `_slot2` fits the grammar; `2slot` does not. The reference does not define the complete character repertoire meant by letter, so Unicode identifier support is unresolved.

Keywords are case-insensitive and cannot be ordinary identifiers. The identifier page does not independently specify every identifier comparison rule. Folio's ASCII-insensitive script/filename matching must not be generalized to all identifiers or filesystem behavior without evidence.

Sources: [Identifier Reference][identifier] revision 25377; [Keyword Reference][keywords] revision 12664; [Script File Structure][structure] revision 25811.

### Reserved keywords

The complete Skyrim reserved-word inventory in the reviewed Keyword Reference is:

| | | | | |
| --- | --- | --- | --- | --- |
| `As` | `Auto` | `AutoReadOnly` | `Bool` | `Else` |
| `ElseIf` | `EndEvent` | `EndFunction` | `EndIf` | `EndProperty` |
| `EndState` | `EndWhile` | `Event` | `Extends` | `False` |
| `Float` | `Function` | `Global` | `If` | `Import` |
| `Int` | `Length` | `Native` | `New` | `None` |
| `Parent` | `Property` | `Return` | `ScriptName` | `Self` |
| `State` | `String` | `True` | `While` | |

Hidden and Conditional are standard declaration flags, not entries in that inventory. Project-defined flags do not automatically become expression keywords. Later games have different inventories; see the dialect guide.

### Whitespace and line boundaries

Whitespace separates tokens but cannot split a keyword, operator, or literal. `==` is equality; `= =` is two tokens. Line endings delimit statements.

A backslash at the end of the code portion continues a statement onto the next line. A comment may follow it. A backslash inside a line comment has no continuation effect.

```papyrus
Int total = 3 + 4 \
    + 5
Int other = 2 ; this comment does not continue the line \
```

Source encoding, every supported line-ending byte sequence, and end-of-file recovery are not specified by this page. Those are frontend contracts to document when established.

### Comments and documentation

| Form | Delimiters | Behavior |
| --- | --- | --- |
| Line comment | `;` through the line ending | Consumes remaining text, including apparent continuation characters |
| Block comment | `;/` through `/;` | May consume multiple lines |
| Documentation comment | `{` through `}` | May consume multiple lines; attaches to specified declarations |

Script File Structure restricts documentation comments to the line following a script header, property declaration, or function declaration. Its list does not explicitly include events. Original-compiler event documentation acceptance is separate from any Folio extension.

The reviewed page does not specify nested block/documentation comments. Do not infer a nesting grammar. CST preservation of comment tokens does not make comments in arbitrary declaration positions valid build input.

### Literals

Skyrim has Bool, Int, Float, String, and None literals. Negative numeric notation and unary negation interact; the signed Int minimum must not first overflow a positive intermediate.

| Kind | Documented form | Value constraints |
| --- | --- | --- |
| Bool | True, False | Keyword case rules apply |
| Decimal Int | Digits, optionally preceded by `-` | Signed 32-bit: -2,147,483,648 through 2,147,483,647 |
| Hex Int | `0x` followed by hexadecimal digits | A–F are case-insensitive; complete signed bit-pattern/overflow handling is not explained |
| Float | Digits, `.`, digits, optionally preceded by `-` | IEEE single precision; decimal values may round |
| String | Text in double quotes | Special characters use escapes |
| None | None | Absent object/array reference in the applicable type context |

The Float grammar does not include exponents, suffixes, or omitted digits beside the decimal point. Scientific notation in explanatory prose is not source-syntax evidence. A suffix in a Statement Reference example conflicts with this grammar.

The listed positive Float range does not fully enumerate signed values, zero, subnormals, infinities, NaNs, or overflow behavior. Do not present that positive minimum as the smallest magnitude of every runtime value.

Strings cannot directly contain a newline, unescaped quote, or unescaped backslash. The documented escape set is:

| Escape | Value |
| --- | --- |
| `\n` | Newline |
| `\t` | Tab |
| `\\` | Backslash |
| `\"` | Double quote |

Unknown escape recovery, Unicode escape syntax, and string encoding are not defined by this set. They must not be borrowed silently from another language.

Sources: [Literals Reference][literals] revision 25539; [Statement Reference][statements] revision 26036.

## Script declarations and visibility

### Script structure

After the header, imports, variables, properties, states, functions, and events may appear in any order. Script members may be used before textual declaration. Function/event locals must be declared before use.

```papyrus
ScriptName Gauge

Int Function Read()
    Return stored
EndFunction

Int stored = 7
```

This illustrates member declaration order, not arbitrary script-level initialization expressions.

Extends establishes one parent script. A child's matching function/event name requires a matching return type and parameter list. The references do not fully explain whether matching includes parameter names/defaults or all modifiers.

Sources: [Script File Structure][structure] revision 25811; [Function Reference][functions] revision 25052.

### Imports

Import ScriptName permits calls to that script's global functions without a script prefix. It does not create an instance or import instance members as globals.

An explicitly qualified global call selects the owning script. Ambiguous imported names require qualification. The page does not supply a complete precedence algorithm among current-script, inherited, and imported candidates.

### Variables and block scopes

Script variables have optional literal initializers and applicable flags. Locals have optional expression initializers. New is therefore not a Skyrim script-variable initializer.

Script variables are private storage; properties provide external access. Availability within inheritance must not be confused with public property visibility.

A local's scope is its function/block. Independent sibling blocks may reuse a name; that name is unavailable after leaving the block. Separate functions may reuse local names.

Variable Reference disallows a function local matching a script variable. Statement Reference distinguishes unrelated blocks from parent/child nested blocks for conflicts. These rules do not permit arbitrary nested shadowing.

```papyrus
If enabled
    Int result = 1
Else
    Int result = 2
EndIf
; result is not in scope here
```

A complete collision matrix involving parameters, functions, properties, inherited members, and flags is not supplied. Partial examples must not become an invented namespace algorithm.

Sources: [Variable Reference][variables] revision 26131; [Statement Reference][statements] revision 26036.

## Types, defaults, and storage

Skyrim types include Bool, Int, Float, String, script objects, and one-dimensional arrays of non-array elements. Omitting a function return type means no returned value; it does not declare a general Void value type.

| Type | Default | Assignment/passing |
| --- | --- | --- |
| Bool | False | Copies a value |
| Int | 0 | Copies a value |
| Float | 0.0 | Copies a value |
| String | `""` | Copies a language value; engine caching still affects casing |
| Script object | None | Copies a reference |
| Array | None | Copies a reference |

Scalar assignment does not cause subsequent changes to one variable to update another. Object/array assignment does not clone referenced storage. Shared array mutations are visible through every reference.

```papyrus
Int[] first = New Int[2]
Int[] second = first
second[1] = 9
; first[1] also observes 9
```

None is not an unresolved symbol or an analyzer error type. Failed name lookup must not produce a valid None expression.

Sources: [Default Value Reference][defaults] revision 9273; [Variable Reference][variables] revision 26131; [Array Reference][arrays] revision 24877.

### Strings and runtime casing

Literals Reference describes case-insensitive strings and a shared engine string cache that retains an encountered spelling. Source literal spelling can therefore differ from later displayed runtime spelling.

This is engine behavior, not justification for normalizing every source string during analysis. A compiler preserves the decoded source value; it cannot reproduce an unknown cache history.

String equality and documented array String searches are case-insensitive. Exact log casing is not a semantic equality test. Cache lifetime/save interactions belong in the runtime guide.

## Casts and conversions

An explicit cast is expression As Type. Implicit conversion is contextual: an explicit conversion's existence does not authorize every mixed-type operation.

The matrix summarizes Cast Reference. Identity means ordinary same-type use, not independently verified acceptance of every redundant As spelling. U/D denote related ancestor/descendant types.

| Source | To Bool | To Int | To Float | To String | To object | To different array type |
| --- | --- | --- | --- | --- | --- | --- |
| Bool | Identity | Explicit | Explicit | Implicit/explicit | Not described as valid | Invalid |
| Int | Implicit/explicit | Identity | Implicit/explicit | Implicit/explicit | Not described as valid | Invalid |
| Float | Implicit/explicit | Explicit | Identity | Implicit/explicit | Not described as valid | Invalid |
| String | Implicit/explicit | Explicit | Explicit | Identity | Not described as valid | Invalid |
| Object | Implicit/explicit | Not described as valid | Not described as valid | Implicit/explicit | D→U implicit; U→D explicit | Invalid |
| Array | Implicit/explicit | Not described as valid | Not described as valid | Implicit/explicit | Invalid | Invalid |

Handle None with its type/expression context, not as a normal script class. The cast tables do not enumerate every standalone None cast; default/array references establish absent-reference usage.

### Conversion results

| Conversion | Documented result |
| --- | --- |
| Int→Bool | Zero is false; other values true |
| Float→Bool | Tests nonzero using an unspecified small epsilon |
| String→Bool | Empty is false; nonempty true |
| Object→Bool | None is false; other references true |
| Array→Bool | True for length at least one; false for zero length |
| Bool→Int | False→0; True→1 |
| Float→Int | Truncates fractional part toward zero |
| String→Int | Parses an integer representation; 0 if no representation is found |
| Bool→Float | False→0.0; True→1.0 |
| Int→Float | Converts to the corresponding single-precision value |
| String→Float | Parses a float representation; 0.0 if no representation is found |
| Scalar→String | Textual value |
| Object→String | Engine object description |
| Array→String | Bracketed element listing, possibly truncated |

The String→Int example establishes acceptance of a numeric prefix followed by text. Whitespace, sign, hexadecimal, exponent, overflow, locale, and exact Float→String precision are not completely specified.

Object downcasts may fail and produce None. Ancestry restricts conversion; sibling types do not become directly convertible merely because they share a parent. A valid conversion through a common ancestor is a separate cast sequence.

Array element compatibility does not authorize array covariance. Child[] cannot be converted to Parent[] using element casts; copying into newly allocated storage is a different operation.

Sources: [Cast Reference][casts] revision 24921; [Arrays (Papyrus)][array-concepts] revision 24878.

## Expressions and operators

Expressions combine literals, variables, calls, members, indexes, array construction, casts, and operators. Parentheses group expressions and delimit arguments in call context.

### Precedence

Highest to lowest:

| Level | Operations |
| --- | --- |
| 1 | Parenthesized expressions; atom/call/index construction |
| 2 | `.` member/call chains |
| 3 | `As` |
| 4 | Unary `-`, `!` |
| 5 | `*`, `/`, `%` |
| 6 | Binary `+`, `-` |
| 7 | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| 8 | `&&` |
| 9 | `\|\|` |
| 10 | Statement assignment: `=`, `+=`, `-=`, `*=`, `/=`, `%=` |

Expression grammar defines array/member chains; brackets are not an arbitrary binary operator. Assignment is a statement despite its appearance in the precedence table. This does not establish value-yielding nested assignment expressions.

Repeated binary grammar groups suggest left-folded chains but do not explicitly explain all associativity. The cast grammar permits one suffix at that level, and unary grammar one operator there. Parenthesized subexpressions allow nesting. Broader acceptance needs explicit dialect/Folio policy.

### Operands and results

| Operation | Baseline behavior | Constraint/caution |
| --- | --- | --- |
| Numeric `+`, `-`, `*`, `/` | Arithmetic | Apply contextual numeric conversions; see Bool coercion caveat below |
| `%` | Integer remainder | Float remainder is not specified |
| String `+` | Concatenation | Conversion differs from numeric arithmetic |
| Unary `-` | Numeric negation | Unary plus is absent from the reviewed unary grammar |
| `!` | Logical negation | Bool conversion applies |
| `&&`, `\|\|` | Bool conjunction/disjunction | Short-circuiting applies |
| Comparisons | Bool result | Float uses unspecified epsilon; full operand matrix absent |
| `=` | Writes assignable storage/property/element | Requires type compatibility and writable destination |
| Compound assignment | Reads, combines, writes | Full property uses both Get/Set; CK rejects array elements |

Integer division discards the remainder. The negative remainder example establishes that remainder can be negative. Divide/remainder by zero produces an unspecified result and engine log error; no guaranteed fallback value is supplied.

Cast Reference includes numeric/Bool arithmetic in which the numeric side auto-converts to Bool rather than Bool auto-converting to a number. The source describes a subsequent explicit numeric conversion. This is evidence of unusual coercion behavior, not a complete specification of arithmetic on every mixed pair; the table must not be read as categorically prohibiting Bool-containing arithmetic. A full operator compatibility/coercion matrix remains an evidence and implementation-audit task.

Bool convertibility does not itself authorize None comparison. Conversion eligibility, operator typing, and target operation semantics need separate checks.

### Evaluation and side effects

`&&` skips its right operand when the left is false; `||` skips it when the left is true. Skipped calls must not run eagerly during lowering.

```papyrus
If candidate != None && candidate.IsReady()
    candidate.Run()
EndIf
```

These are hypothetical script members demonstrating guarded evaluation, not implied engine APIs.

The language pages do not completely specify side-effect ordering of all calls, receivers, indexes, and destinations. Existing Folio evaluation contracts appear separately below rather than being attributed to CK.

Sources: [Expression Reference][expressions] revision 25012; [Operator Reference][operators] revision 25698; [Cast Reference][casts] revision 24921.

## Statements and local lifetime

Baseline statements include declaration, assignment, return, conditional, loop, and expression/call statements. The reviewed set does not list For, Break, Continue, or switch statements. Other games' additions are not Skyrim syntax by default.

### Declarations and assignment

A local optionally receives an expression initializer; otherwise the type default applies. Script initializers are more restricted.

Assignment needs a writable destination. A property without Set cannot be assigned; without Get it cannot be read. Compound assignment needs both accessors.

Statement Reference's assignment prose incorrectly points to the left expression when describing calculation. Its examples/operator page identify the value expression on the right; that wording must not reverse assignment semantics.

### Return

Return immediately leaves the function/event. A typed function returns a compatible expression; a function without a return type or an event uses bare Return.

Unconditionally following statements do not execute. Whether unreachable source still receives diagnostics is a separate analyzer policy.

Typed fallthrough has conflicting descriptions; do not promise a type default or None. See [Missing return values](#missing-return-values).

### Conditionals and loops

If tests its condition, then ElseIf conditions until one branch is selected. Else supplies fallback. EndIf closes the statement.

While tests before its body and before subsequent iterations; an initially false condition runs no body. EndWhile closes it. Conditions use applicable Bool conversions.

### Loop-local lifetime

Statement Reference describes an uninitialized local inside While as retaining its value across iterations rather than resetting whenever its declaration is encountered. Lexical scope and storage initialization lifetime must be distinguished.

```papyrus
Int iteration = 0
While iteration < 3
    Int accumulated
    accumulated += 1
    iteration += 1
    ; accumulated progresses across iterations
EndWhile
```

An explicit initializer is executable source when a value must reset each iteration. The passage does not define all VM storage lifetimes/control-flow joins.

The source page's illustrative loop omits its required counter increment. This original example avoids that teaching defect.

Source: [Statement Reference][statements] revision 26036.

## Functions, events, and inheritance

### Function declarations

A header contains an optional return type, Function, name, parameters, and applicable modifiers/flags. Non-native functions have bodies ending with EndFunction. Native declarations have no body or closing terminator.

Same-scope ordinary functions cannot reuse a name as overloads. State implementations follow the separate state contract.

Overrides must match parent return type and parameters. Default/name equivalence and every Global/Native combination are not fully established by these descriptions. Function Reference also prohibits specifying the same modifier more than once.

### Parameters and arguments

A parameter has a type, name, and optional constant default. After the first defaulted parameter, every remaining parameter must have a default.

```papyrus
Int Function Offset(Int value, Int increment = 2)
    Return value + increment
EndFunction
```

Positional arguments follow declaration order. `parameterName = expression` binds by declared name. Optional argument defaults are inserted at the call site; changing only the callee declaration does not make an already compiled caller adopt a changed default.

```papyrus
Int a = Offset(6)
Int b = Offset(6, increment = 4)
```

Named arguments may be out of declaration order. The page does not fully define mixed positional/named ordering, duplicate bindings, or unknown names. These need precise static acceptance rules rather than guessed intent.

### Instance and global calls

An instance call operates on a script instance. An omitted receiver denotes the current instance where permitted. Global functions belong to a script type, have no Self context, and access instance members through supplied objects.

An external global call requires the owning script prefix unless imported and unambiguous. Self exists only in non-global contexts.

Parent is a special parent-call receiver, not a general variable for storage, return, or arbitrary property access. It requires inheritance context.

### Parent dispatch

The basic description says Parent bypasses the local override. A later Function Reference section describes a more specific engine observation: each Parent call advances one inheritance level, with an implementation found farther up executing in the nearer level's context.

In a chain A←B←C←D where B/D implement a function, C does not, and the overrides call Parent, B's implementation can execute twice. Directly calling the nearest ancestor declaration therefore does not alone establish engine-equivalent dispatch.

This is sourced runtime behavior with an external verification requirement. State interactions and target differences belong in [Runtime semantics](papyrus/runtime.md), not an inferred lexical lookup algorithm.

### Events and native binding

Events have parameters, no return type, and no Global modifier. Native events omit their bodies. Otherwise their special variables match non-global functions.

Events can be called like functions. An arbitrary declaration does not make the engine send an event; engine-originated calls require the expected name/signature. This is not an event catalog.

Native on an unbound name may pass CK compilation but fail at runtime. Declaration visibility does not prove engine binding availability.

Sources: [Function Reference][functions] revision 25052; [Events Reference][events] revision 25007; [Script File Structure][structure] revision 25811.

## Properties and declaration flags

### Full properties

A full property ends with EndProperty and contains Get and/or Set. At least one is required.

| Accessor | Signature | Operation |
| --- | --- | --- |
| Get | No parameters; property return type | Reading calls it |
| Set | One property-type parameter; no return type | Assignment passes the value |

Missing Get forbids reading; missing Set forbids writing. A property is public access behavior, not necessarily direct backing storage. Accessors can have side effects.

```papyrus
Int stored
Int Property Current
    Int Function Get()
        Return stored
    EndFunction
    Function Set(Int next)
        stored = next
    EndFunction
EndProperty
```

Property Reference lists Hidden as the only applicable full-property flag. CK visibility also depends on having Set and not being Hidden.

### Generated properties

Auto generates storage and read/write access. Its initializer, if present, is constant rather than a call/general expression.

AutoReadOnly requires constant initialization and is not assignable in-game. Property Reference says its value is not baked into a save; this persistence distinction also belongs in the runtime guide.

```papyrus
Int Property Threshold = 4 Auto
String Property Label = "Meter" AutoReadOnly
```

Auto/AutoReadOnly are alternate property forms, not unrelated metadata bits.

### Standard flags

| Flag/modifier | Location | Meaning/prerequisites |
| --- | --- | --- |
| Hidden | Script/property | Hides from applicable CK list/window; not property privacy |
| Conditional | Script | Condition visibility; CK permits only one Conditional script on an object |
| Conditional | Script variable | Exposes condition-system storage; owner must be Conditional |
| Conditional | Auto property | Marks backing storage; owner must be Conditional |
| Auto | Property | Generated writable storage/access |
| AutoReadOnly | Property | Generated initialized read-only access |
| Auto | State | Initial state; separate syntactic role |
| Global | Function | No implicit instance context |
| Native | Function/event | Runtime-supplied implementation |

The flags page's broad claim that flags do not affect compilation is too coarse for all entries. Property forms, callable modifiers, state selection, and user metadata require distinct semantic roles.

Conditional AutoReadOnly is disputed and is not asserted valid here. See [Unresolved descriptions](#unresolved-descriptions).

Sources: [Property Reference][properties] revision 25738; [Flag Reference][flags] revision 10064.

### User flag files and language modifiers

CK compilers load user-flag definitions from `.flg` files. These differ from reserved language modifiers such as Auto, AutoReadOnly, Global, and Native. A user flag has a name, a bit index, and declaration applicability; its name does not become a reserved keyword.

The supplemental [Open Papyrus flag grammar][openpapyrus-grammar], commit `39065b24e61a66c070f20d86c5f669490cfa6ef1`, defines numeric flag entries and applicability blocks. Script, Property, Variable, and Function are the relevant baseline declaration categories; its additional StructVar/Group/composite forms must not be imported into Skyrim without dialect evidence. The grammar alone does not establish the numeric upper bound.

Supplemental [Open Papyrus maintainer documentation][openpapyrus-flags] shows Skyrim Hidden at bit 0 for Script/Property, and Conditional at bit 1 for Script/Variable. Conditional on an Auto property affects generated backing storage, explaining why its flag-file scope is Variable rather than Property.

A pinned [TESV SDK flag-file transcription][tesv-flags] documents indexes from 0 through 31, ignored whitespace, and applicability to all four categories when the declaration omits a scope block. Its Hidden/Conditional entries agree with the scopes above. This is SDK-transcription evidence; the selected original compiler and flag file still require independent validation.

An original illustrative definition, not a redistributed CK flag asset:

```text
Flag ProjectTag 7
{
    Script
    Function
}
```

This supplemental evidence comes from a compiler project, not Bethesda's language-reference category. Modern composite flags and new applicability kinds belong in the dialect guide.

Folio's manifest-defined `user-flags` model supports explicit indexes and declaration scopes, deterministic allocation for omitted indexes, and separate property/backing-storage routing. It does not load `.flg` files or claim equivalence to the original compiler's flag-file parser. See [manifest configuration](architecture/project-model.md#manifest).

The current [Folio target profile](architecture/compiler.md#target-lowering-and-mir) documents maximum user-flag bit 31. That implementation parameter is not, by itself, evidence for the original CK compiler's complete flag validation rules.

## States

A named state contains function/event implementations and ends with EndState. Auto may precede State. Implementations outside named states belong to the empty state.

Each named-state function requires an identically named, typed, and parameterized empty-state declaration in the same script or a parent. A state supplies alternate implementations of a callable contract rather than unrelated overloads.

A script has at most one Auto state. A child's selection overrides its parent's; otherwise the parent's selection applies. Initial Auto entry does not send OnBeginState.

```papyrus
Int Function ReadMode()
    Return 0
EndFunction

Auto State Active
    Int Function ReadMode()
        Return 1
    EndFunction
EndState
```

GotoState(String) switches state; an empty string selects empty state. The destination need not exist on the current script. Rejecting every undeclared literal state would be a separate Folio policy.

GetState returns the current name as String. A state's missing implementation may fall back; it does not automatically disable a callable.

State Reference's dispatch list duplicates a parent-state step. The complete hierarchy/empty-state order is unresolved from that list alone; see runtime dispatch evidence in the companion guide.

It reports a CK/engine load limit of 128 states including empty state, leaving 127 named states. Inherited-state counting is unspecified. Folio enforces the local limit in its Skyrim target; this is not a universal Papyrus bound or proof of engine loading behavior.

Source: [State Reference][states] revision 26035.

## Arrays and built-in array operations

### Types and construction

T[] is an array of a non-array element type. Arrays may appear as parameters, returns, variables, or properties. Skyrim has no nested-array type such as Int[][].

New T[n] requires an integer literal from 1 through 128. A variable/computed expression is not an accepted Skyrim constructor size. New creates arrays, not script object instances.

Construction belongs inside functions/events, not arbitrary script initializers. Elements receive type defaults. CK-populated array properties can obtain contents through editor data.

The constructor bound does not prove a universal VM array capacity: extension allocation can create larger arrays without changing this grammar. No extension API inventory is needed for that distinction.

### References, indexing, and Length

Array assignment/passing shares storage. The concepts page describes collection after references disappear, without exact reclamation timing.

Indexes are zero-based, through Length minus one. They can be expressions. Static typing and runtime bounds are different checks.

Length is read-only Int. None.Length returns 0. That safe query does not make None indexing safe or establish a default value for every invalid access.

### Find and RFind

The concepts page provides these element-dependent intrinsic signatures. T means the receiver's element type, not a source type parameter:

```text
Int Find(T akElement, Int aiStartIndex = 0)
Int RFind(T akElement, Int aiStartIndex = -1)
```

| Method | Search | Result |
| --- | --- | --- |
| Find | Supplied start inclusive, default 0, toward final element | First match in that direction |
| RFind | Supplied start backward toward 0; default -1 means last element | First backward match |

Failure is described as a negative index, without an exact sentinel in this passage. Use `< 0` unless exact-value evidence is established elsewhere.

The needle must be compatible with the element type. Examples establish String case-insensitive matching and rejection of an unrelated script type. Float epsilon matching, every identity case, invalid start indexes, and searching None are not completely defined.

```papyrus
String[] tags = New String[3]
tags[0] = "blue"
tags[2] = "BLUE"
Int first = tags.Find("Blue")
Int last = tags.RFind("blue")
; documented String matching finds 0 and 2
```

Intrinsics do not make all engine scripts visible. Shared authoritative signatures should drive analysis, lowering, and editor descriptions.

Sources: [Array Reference][arrays] revision 24877; [Arrays (Papyrus)][array-concepts] revision 24878; [Cast Reference][casts] revision 24921.

## Compiler restrictions, defects, and unresolved descriptions

### CK array compilation defects

Array/Operator references attribute array-element compound assignment rejection to the original compiler's inability to handle it. This is a compiler restriction, not independent proof of VM impossibility. A later compiler can select equivalent lowering with explicit, verified semantics.

Array Reference also reports incorrect compilation of complex Int-array indexes and Find calls embedded inside indexes. These are defects, not a language ban on expression indexes. The CK workaround is to compute the index into a local first.

Correct Folio behavior must be evaluated independently of corrupted original output. Known compiler defects are not a reason to reproduce unwanted mutation or repeated evaluation.

### Missing return values

Statement Reference describes typed fallthrough as returning None with a possible engine warning. Function Reference's Bugs section instead reports indeterminate results and cascading failures, proposing stack corruption as a possible cause.

The result is unresolved and unsafe. A compiler may diagnose it strictly, but acceptance and reachable-path rules must be documented separately. This update does not establish such a policy as verified Folio behavior.

### Unresolved descriptions

| Issue | Evidence problem | Handling |
| --- | --- | --- |
| Conditional AutoReadOnly | Flag page limits Conditional to Auto; Property syntax also lists AutoReadOnly | Preserve dispute; seek compiler/engine evidence |
| State fallback | Repeated parent-state step | Do not silently correct into a normative algorithm |
| Float epsilon | No exact threshold/formula | Preserve engine operation; do not invent a constant |
| Float suffix | Statement example differs from literal grammar | Example does not extend grammar |
| Subtraction spacing | `x-1` rejection despite general whitespace guidance | Old compiler parse compatibility behavior |
| Unary plus | Missing from expression unary grammar | Later Folio hover/policy review needed |
| Identifier characters | Letter repertoire absent | Separate ASCII/Unicode policy from source evidence |
| Typed fallthrough | Conflicting return descriptions | No guaranteed value |
| Mixed operators | Coercion examples, no full matrix | Do not invent symmetric promotion |
| Search failure | Negative index, exact value absent | Do not strengthen to a precise sentinel |

The runtime guide carries additional Parent, hook, scheduling, persistence, and invalid-operation questions. The dialect guide records later games' changes.

## Folio behavior boundary

This section describes existing implementation behavior, separately from reference requirements. Focused pure unit tests cover source/declaration validation, semantic contracts, target representation, and editor models. The [conformance roadmap](planning/roadmap.md#papyrus-conformance) retains unresolved language evidence and external verification; these tests do not establish complete conformance or real-engine compatibility.

### Existing documented behavior

- Recognizes scripts, imports, variables, properties, states, functions/events, parameter defaults, Global/Native, and custom flags.
- Bodies support declarations, Return, assignments, conditionals, While, calls, literals, arithmetic/comparison/logical operations, members, arrays, and casts.
- Script/filename matching is ASCII-insensitive; same-package duplicate scripts are errors.
- Lossless CST retains whitespace, comments, continuations, and errors. Recoverable editor facts do not authorize artifact generation.
- Analysis resolves inheritance, types, members, calls, argument binding, and source locations. Unresolved names remain errors. Dependencies expose APIs without providing runtime implementations.
- Conditions and logical operands accept documented Bool conversion from Int, Float, String, object/array references, and None. Analysis records conversions; lowering emits MIR Cast operations.
- Bool conversion does not relax None comparisons for numeric/Bool/String operands. Array conditions and None comparisons are not assumed equivalent.
- String + and String += accept Int/Float conversion, lowered through string Cast/StrCat. Float remainder is rejected rather than approximated.
- GetState/GotoState are generated instance operations. Previously documented transition order is OnEndState, update, OnBeginState; runtime caveats require separate evidence.
- Length/Find/RFind use target array operations. Other native methods require visible declarations.

### Declaration and target checks

- ScriptName must be the first non-comment declaration and occur once. Reserved keywords cannot be declaration identifiers. Continuations retain trailing comments. Casts consume a type reference at the documented precedence; equality and relational comparisons share a precedence level.
- Lexical blocks constrain local visibility and conflicts. Distinct sibling locals retain distinct storage identities. Local names cannot conflict with script variables; uninitialized loop locals do not receive per-iteration resets.
- Literal defaults are checked even on unused declarations. Declaration constants require matching types, Int-to-Float widening, or None for object/array references. Runtime assignments, arguments, and returns support implicit String conversion; explicit object casts require an ancestry relationship.
- Full properties require valid Get and/or Set accessors. Reads and writes require the respective accessor, and compound assignment requires both. Auto and AutoReadOnly are exclusive; AutoReadOnly requires a literal and emits a constant getter without mutable backing storage. Inherited properties cannot be redeclared.
- Standard/custom flags are checked by declaration site and owner prerequisites; duplicate modifiers are errors. Conditional on Auto storage is written to the generated variable. Custom metadata follows its registered scopes and bit allocation.
- Parent is restricted to instance-call receivers and emits CALLPARENT. Named-state methods require an empty-state contract locally or in ancestors, except implicit OnBeginState/OnEndState callbacks. OnInit requires an explicit empty-state declaration. The local target limit is 128 states including the empty state.
- Find/RFind parameter names are `akElement` and `aiStartIndex`; GotoState uses `asNewState`. Semantic binding, signature help, and generated methods share these names.
- Root-source analysis and PSC declaration extraction share declaration checks. Dependency body compilation is excluded; malformed declaration facts and nonliteral initializers are rejected rather than discarded.

### Explicit Folio policies

Event documentation is accepted as an extension alongside documented script/property/function header documentation. State, variable, inline, and statement documentation is rejected. Identifiers use ASCII letters/digits/underscore. Expression unary plus and repeated unary/casts, `\r` string escapes, and reopened named states remain Folio extensions without established CK equivalence; reopened states merge their API declarations.

Integer decoding uses the signed Int range for decimal and hexadecimal forms, including the signed minimum. Full-width unsigned hexadecimal bit patterns are rejected pending compiler evidence. Conditional AutoReadOnly and typed-function fallthrough are rejected conservatively. The complete mixed-operand matrix and other unresolved cases remain in the roadmap; these policies must not be presented as settled original-compiler rules.

### Argument filling extension

Default policy reports required-argument omission as `semantic.argument-count`. `fill-missing-arguments` supplies the type default at the call site and reports `semantic.argument-defaulted`.

This is Folio policy, separate from declared optional parameters. It does not establish that CK permits missing required arguments.

Named bindings preserve source evaluation order, including supplied arguments around filled parameters. PEX APIs may have unknown defaults/callable kinds; filling does not guess unknown facts. See [Experimental PEX dependencies](architecture/project-model.md#experimental-pex-dependencies).

### Existing evaluation and build contracts

[Compiler pipeline](architecture/compiler.md#evaluation-order) documents short-circuit lowering, source-order named argument evaluation, single evaluation of compound-assignment receiver/array/index, preservation of the old destination before the right side, and capture of a binary left result before later side effects overwrite storage.

Static review found explicit lowering paths for these contracts; focused probes also confirmed that uninitialized loop-local declarations do not emit per-iteration reset assignments. Remaining validation belongs in the [conformance roadmap](planning/roadmap.md#coverage-and-unresolved-decisions). This does not establish real-engine equivalence or require reproducing CK compiler defects.

A build requires semantic, target, and MIR validation. `check` does not perform final PEX encoding; `build` can report further layout limits. See [Compiler pipeline](architecture/compiler.md) and [Projects and dependencies](architecture/project-model.md).

## Source coverage

Revisions identify reviewed evidence, not supported releases. All 18 Skyrim language-reference entries are accounted for below. Runtime/cross-game evidence has companion coverage records.

| Reference | Revision | Coverage |
| --- | --- | --- |
| [Array Reference][arrays] | 24877 | Types, construction, indexes, Length, defects |
| [Cast Reference][casts] | 24921 | Conversion matrix, results, ancestry, arrays |
| [Default Value Reference][defaults] | 9273 | Type defaults |
| [Events Reference][events] | 25007 | Events, modifiers, engine signature boundary |
| [Expression Reference][expressions] | 25012 | Expression grammar, grouping, unary/casts |
| [Flag Reference][flags] | 10064 | Standard applicability and disputes |
| [Function Reference][functions] | 25052 | Functions, defaults, Parent, return defects |
| [Identifier Reference][identifier] | 25377 | Identifier form and character uncertainty |
| [Keyword Reference][keywords] | 12664 | Complete Skyrim keyword inventory |
| [Language Reference Notation][notation] | 12708 | Grammar interpretation; no executable feature |
| [Literals Reference][literals] | 25539 | Literal forms, widths, escapes, casing |
| [Operator Reference][operators] | 25698 | Operators, precedence, failures, restrictions |
| [Property Reference][properties] | 25738 | Accessors, generated forms, persistence |
| [Return (Papyrus)][return] | 15018 | Redirect to Statement; no independent semantics |
| [Script File Structure][structure] | 25811 | Headers, declarations, whitespace, comments |
| [State Reference][states] | 26035 | State contract, Auto, count limit |
| [Statement Reference][statements] | 26036 | Statements, returns, loop-local lifetime |
| [Variable Reference][variables] | 26131 | Initialization, scopes, copying/references |

Supplemental [Arrays (Papyrus)][array-concepts], revision 24878, supplies Find/RFind signatures, starts, String matching, and reference-sharing facts. Its tutorial descriptions are not an authoritative VM memory-layout specification.

Supplemental [Open Papyrus flags documentation][openpapyrus-flags] and its [flag grammar][openpapyrus-grammar] distinguish user metadata from language modifiers and supply the standard Skyrim flag scope/index evidence. The grammar is pinned to commit `39065b24e61a66c070f20d86c5f669490cfa6ef1`; the documentation page has no CK revision identifier. These are maintainer/compiler-project sources, not additional Bethesda reference pages.

The supplemental [TESV SDK flag-file transcription][tesv-flags] is pinned to skymp commit `47ea52f54de40ea1319bd8bb7c1d2a4c485c6364`. It supplies the 0–31 index range and omitted-scope behavior, without establishing every original compiler diagnostic.

[Category:Papyrus][category], revision 22915, supplies discovery links to language, compiler, and runtime domains rather than an additional grammar.

[Source notation][notation] uses quoted terminals, named nonterminals, optional brackets, grouping parentheses, repetitions, and a rule-definition separator. The local examples summarize behavior rather than copying full wiki productions or tutorial programs.

[arrays]: https://ck.uesp.net/wiki/Array_Reference
[casts]: https://ck.uesp.net/wiki/Cast_Reference
[defaults]: https://ck.uesp.net/wiki/Default_Value_Reference
[events]: https://ck.uesp.net/wiki/Events_Reference
[expressions]: https://ck.uesp.net/wiki/Expression_Reference
[flags]: https://ck.uesp.net/wiki/Flag_Reference
[functions]: https://ck.uesp.net/wiki/Function_Reference
[identifier]: https://ck.uesp.net/wiki/Identifier_Reference
[keywords]: https://ck.uesp.net/wiki/Keyword_Reference
[notation]: https://ck.uesp.net/wiki/Language_Reference_Notation
[literals]: https://ck.uesp.net/wiki/Literals_Reference
[operators]: https://ck.uesp.net/wiki/Operator_Reference
[properties]: https://ck.uesp.net/wiki/Property_Reference
[return]: https://ck.uesp.net/wiki/Return_(Papyrus)
[structure]: https://ck.uesp.net/wiki/Script_File_Structure
[states]: https://ck.uesp.net/wiki/State_Reference
[statements]: https://ck.uesp.net/wiki/Statement_Reference
[variables]: https://ck.uesp.net/wiki/Variable_Reference
[array-concepts]: https://ck.uesp.net/wiki/Arrays_(Papyrus)
[category]: https://ck.uesp.net/wiki/Category:Papyrus
[openpapyrus-flags]: https://open-papyrus.github.io/docs/Papyrus_Language_Reference/Lexial_structure/Flags.html
[openpapyrus-grammar]: https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/FlagsParser.g4
[tesv-flags]: https://github.com/skyrim-multiplayer/skymp/blob/47ea52f54de40ea1319bd8bb7c1d2a4c485c6364/cmake/TESV_Papyrus_Flags.flg
