# Papyrus dialects and future targets

This reference records the language changes after Skyrim and separates documented
Fallout 4 rules from compiler-derived Starfield evidence and Fallout 76 unknowns.
The shared language baseline is [Papyrus language reference](../papyrus.md);
execution, persistence, and engine interaction are discussed in
[Runtime semantics](runtime.md). Folio currently exposes the `skyrim` dialect and
`skyrim-se` target only. The information here prepares later development; it does
not establish support for another source dialect, compiler, game, or PEX format.

## Evidence and compatibility model

The Fallout 4 Creation Kit reference is the main source for the changes below.
Its [language category](https://falloutck.uesp.net/wiki/Category:Papyrus_Language_Reference)
contains 20 pages. The readable [Fallout Wiki CK archive](https://fallout.wiki/wiki/Resource:Creation_Kit)
preserves those reference pages, but is a maintained mirror, not a new Bethesda
standard. A mirror revision can contain community corrections. References to
"CK documentation" mean the archived CK description, with its limits and
contradictions preserved rather than silently repaired.

Starfield evidence is weaker. The [syntax inventory](https://starfieldwiki.net/wiki/Starfield_Mod:Papyrus_Syntax_Reference)
identifies `PCompiler.dll` version `4.7.0.5` and explicitly says it is incomplete.
The [OpenPapyrus grammars](https://github.com/fireundubh/OpenPapyrus)
are a maintainer's reconstruction from decompiled compilers, not Bethesda's
published grammar or a tested conformance oracle. The pinned reconstruction has
internal contradictions documented below. Game API declarations reproduced by
the [Papyrus Index](https://papyrus.bellcube.dev/starfield/) are useful supplementary
evidence, but the site's descriptions sometimes link to Fallout 4 pages and its
rendered signatures can flatten special parameter types into `String`.

Keep four separate compatibility decisions: source syntax, static semantics,
available declarations/native bindings, and artifact/VM format. The CK
[migration guide](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4)
says Skyrim and Fallout 4 PEX files are mutually incompatible even where source
needs few changes: the game rejects a mismatched game ID. Recompilation does not
prove that native APIs or event behavior agree.

## Cross-game feature matrix

`Documented` means covered by the Fallout 4 CK reference; `evidence` means the
specified Starfield source recognizes the feature, but completeness and precise
constraints need target-specific validation. `Unknown` never means inherited
Fallout 4 behavior. This matrix concerns Bethesda dialects; third-party language
extensions have a separate section.
[Sources: CK migration guide](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4),
[Starfield syntax inventory](https://starfieldwiki.net/wiki/Starfield_Mod:Papyrus_Syntax_Reference),
[compiler reconstruction](https://github.com/fireundubh/OpenPapyrus/tree/39065b24e61a66c070f20d86c5f669490cfa6ef1).

| Feature | Skyrim | Fallout 4 | Starfield | Fallout 76 |
| --- | --- | --- | --- | --- |
| Namespaced scripts and imports | No | Documented | Evidence | Unknown |
| Script-owned structs | No | Documented | Evidence | Unknown |
| `Var`, `Var[]` | No | Documented | Declaration evidence | Unknown |
| `Is` type test | No | Documented | Evidence | Unknown |
| Property `Group` blocks | No | Documented | Evidence | Unknown |
| Script-level `Native`, `Const` | No | Documented | Evidence | Unknown |
| Const variables and auto properties | No | Documented | Evidence | Unknown |
| `Mandatory`/group editor flags | No | Documented | Requires actual flags file | Unknown |
| Array size expression | No; literal size baseline | Documented | Evidence | Unknown |
| Explicit array-to-array cast | No | Documented | Requires target confirmation | Unknown |
| Array add/insert/remove/clear | No | Documented | Compiler-derived evidence | Unknown |
| Struct-member array search | No | Documented | Compiler-derived evidence | Unknown |
| Custom events | No | Documented | Evidence | Unknown |
| Remote event handler syntax | No | Documented | Evidence | Unknown |
| Per-script event registration | Skyrim registration model | Documented | Declaration evidence | Unknown |
| `DebugOnly`, `BetaOnly` call stripping | No | Documented | Compiler-derived evidence | Unknown |
| `ScriptObject` common base | No | Documented | Declaration evidence | Unknown |
| State callbacks with string arguments | No; zero arguments | Documented API | Declaration evidence | Unknown |
| Named guards and lock blocks | No | No | Compiler-derived evidence | Unknown |
| Guard/access modifiers | No | No | Incomplete compiler inventory | Unknown |
| Matching-structs array operation | No | No | Opcode/reconstruction evidence | Unknown |

## Fallout 4 source organization and names

### Coverage of the CK language-reference category

All 20 category entries were read through the archive. The table records where
each is reconciled; "shared baseline" means the relevant rule remains in
[the main guide](../papyrus.md), with the Fallout 4 difference or evidence defect
identified here. This is category coverage, not proof that every CK article or
every native method constitutes a complete language standard.

| Archived language page | Coverage in this reference |
| --- | --- |
| [Array Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Array_Reference) | Arrays and compiler-provided operations |
| [Cast Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Cast_Reference) | Var, Is, explicit array conversion; struct-unboxing conflict |
| [Default Value Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Default_Value_Reference) | Shared scalar/object defaults; struct and Var default to None |
| [Events Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Events_Reference) | Native-only new ordinary events; custom and remote event signatures |
| [Expression Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Expression_Reference) | Shared precedence plus Is/new struct; stale grammar noted below |
| [Flag Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Flag_Reference) | Flags table, native/const restrictions, conflicts |
| [Function Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Function_Reference) | Shared call/default rules; special-name categories and release flags |
| [Group Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Group_Reference) | Ordering, merge, legal contents |
| [Identifier Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Identifier_Reference) | Qualified names and namespace paths |
| [Keyword Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Keyword_Reference) | Added reserved tokens below |
| [Language Reference Notation](https://fallout.wiki/wiki/Resource:Creation_Kit/Language_Reference_Notation) | Shared notation; this guide uses explicit newlines/metavariables |
| [Literals Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Literals_Reference) | Shared five literal kinds; no first-class runtime type literal |
| [Operator Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Operator_Reference) | Shared operators/short circuit plus Is and struct member access |
| [Papyrus Naming Conventions](https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus_Naming_Conventions) | Style guidance, not a compiler constraint |
| [Property Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Property_Reference) | Shared accessor shapes; auto const/native restrictions |
| [Script File Structure](https://fallout.wiki/wiki/Resource:Creation_Kit/Script_File_Structure) | Top-level forms, imports, header, docstrings |
| [State Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/State_Reference) | Shared state model; updated callbacks; prose defect below |
| [Statement Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Statement_Reference) | Shared define/assign/return/if/while; stale examples below |
| [Struct Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Struct_Reference) | Declaration, limits, initialization, reference identity |
| [Variable Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Variable_Reference) | Scope baseline, Var, reference structs, Const |

The Fallout 4 keyword page adds `BetaOnly`, `Const`, `CustomEvent`,
`CustomEventName`, `DebugOnly`, `EndGroup`, `EndStruct`, `Group`, `Is`,
`ScriptEventName`, `Struct`, `StructVarName`, and `Var` to the Skyrim inventory.
These are case-insensitive reserved tokens; editor flag names such as
`Mandatory` are configured separately. No new `For`/`Break`/`Switch` statement
family is established by the Fallout 4 CK statement page.
[Sources: keywords](https://fallout.wiki/wiki/Resource:Creation_Kit/Keyword_Reference),
[statements](https://fallout.wiki/wiki/Resource:Creation_Kit/Statement_Reference).

The category retains defects copied from earlier documentation: Expression
Reference omits `Is`/struct allocation and shows literal-size array creation,
while Cast/Struct/Array references explicitly add those features. Statement
Reference uses `int var` despite the reserved `Var` keyword, and a `60.0f`
example despite the literal page not defining suffix syntax. State Reference's
fallback list repeats "parent state" where an empty-state fallback is intended.
The operator page also reverses operand prose for ordered comparisons despite
its conventional symbols/examples. Treat these examples/prose as source defects;
do not encode their mistakes into a target grammar or semantic model.
[Sources: expressions](https://fallout.wiki/wiki/Resource:Creation_Kit/Expression_Reference),
[statements](https://fallout.wiki/wiki/Resource:Creation_Kit/Statement_Reference),
[states](https://fallout.wiki/wiki/Resource:Creation_Kit/State_Reference),
[operators](https://fallout.wiki/wiki/Resource:Creation_Kit/Operator_Reference).

Standalone `Type Reference`/`String Script` pages were not established in this
Fallout 4 archive retrieval and are not entries in its 20-page category. The
applicable type rules are distributed across Cast, Variable, Array, Struct,
Function, and Literal pages. No String-method API or String-size limit is inferred
from Skyrim or a missing page; string literal/conversion/comparison rules are
covered by the relevant language pages. The naming-conventions article explicitly
says its examples come from Skyrim, so it cannot impose a mandatory Fallout 4
property prefix or casing convention.
[Sources: category](https://falloutck.uesp.net/wiki/Category:Papyrus_Language_Reference),
[naming conventions](https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus_Naming_Conventions).

### Qualified script and struct names

The CK [Identifier Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Identifier_Reference)
defines a qualified script name as an identifier followed by zero or more
colon-separated identifiers. The final component is the script; preceding
components are namespaces. Namespace components map to folders relative to a
source root, not to runtime object nesting.

```text
identifier       ::= (letter | '_') (letter | digit | '_')*
script-name      ::= identifier (':' identifier)*
external-struct  ::= script-name ':' identifier
```

For example, `Archive:Jobs:Counter` belongs in `Archive/Jobs/Counter.psc` under
the relevant source root. Its struct `Entry` is named
`Archive:Jobs:Counter:Entry`. The same colon syntax can therefore denote a
namespace-qualified script or a struct owned by a script; resolution must use
the declaration context rather than splitting at an arbitrary fixed depth.
[Sources: identifiers](https://fallout.wiki/wiki/Resource:Creation_Kit/Identifier_Reference),
[struct types](https://fallout.wiki/wiki/Resource:Creation_Kit/Struct_Reference).

```papyrus
ScriptName Archive:Jobs:Counter

Struct Entry
  Int Amount = 0
EndStruct

Function CreateEntry()
  Archive:Jobs:Counter:Entry item = new Archive:Jobs:Counter:Entry
  item.Amount = 3
EndFunction
```

The CK [Script File Structure](https://fallout.wiki/wiki/Resource:Creation_Kit/Script_File_Structure)
adds structs, groups, and custom event declarations to the top-level list.
The header remains the first non-comment line. Definitions need not precede
their use at script scope; function locals still require declaration before use.
Namespace syntax does not permit a variable named `Archive:Amount`, or create
namespaces for arbitrary local identifiers.

### Imports

`Import` can import a script's global functions and structs, or a namespace to
shorten script names. Ambiguity still requires qualification. It is a name
resolution facility; it does not instantiate a script or grant native API access.
[Source: imports](https://fallout.wiki/wiki/Resource:Creation_Kit/Script_File_Structure#Imports).

```papyrus
Import Archive:Jobs:Counter

Function MakeEntry()
  Entry item = new Entry
EndFunction
```

### Documentation comment locations

Fallout 4 adds group and struct-member documentation locations. CK documents
documentation comments after script headers, properties, groups, struct members,
and functions. An ordinary script variable does not acquire documentation support
just because struct members have it. Preserve the group/property ordering needed
by editor metadata.
[Sources: comments](https://fallout.wiki/wiki/Resource:Creation_Kit/Script_File_Structure#Documentation_Comments),
[variables](https://fallout.wiki/wiki/Resource:Creation_Kit/Variable_Reference).

## Fallout 4 types, structs, casts, and names

### Struct declaration and allocation

Structs are script-owned reference objects with fields. They do not contain
functions/events or extend scripts/structs. The CK requires at least one member;
members cannot be arrays, `Var`, other structs, or `Const`. Members may have
literal initializers and documentation comments. Allocation is an executable
expression permitted inside functions, not a script-level initializer.
[Source: Struct Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Struct_Reference).

```text
struct-definition ::= 'Struct' identifier newline
                      member-definition+ 'EndStruct' newline
struct-creation   ::= 'New' struct-type
member-access     ::= expression '.' identifier
```

```papyrus
Struct Sample
  Float Weight = 1.0
  {Default weight used for this sample.}
  Quest Owner
EndStruct

Function DemonstrateSharing()
  Sample first = new Sample
  Sample second = first
  second.Weight = 2.0 ; first and second refer to the same Sample
EndFunction
```

These are invalid Fallout 4 member declarations, even though some are valid
script variables or locals:
[Source: struct restrictions](https://fallout.wiki/wiki/Resource:Creation_Kit/Struct_Reference#Defining_a_Struct).

```papyrus
Struct InvalidSample
  Var Payload
  Int[] Counts
  Sample Nested
  Float Limit = 1.0 Const
EndStruct
```

An uninitialized struct variable is `None`. Creating a `Sample[]` creates array
slots containing `None`; each desired struct must be allocated separately. A
struct assignment or argument passes the reference, not a memberwise copy.
Editor-filled struct properties can be constructed by CK.
[Sources: allocation](https://fallout.wiki/wiki/Resource:Creation_Kit/Struct_Reference#Struct_Creation),
[defaults](https://fallout.wiki/wiki/Resource:Creation_Kit/Default_Value_Reference).

### `Var`

`Var` stores a value and its runtime type. CK describes it as usable through
type tests and casts rather than directly exposing the stored value's members or
arithmetic. Scalars, script references, and structs may be stored in it; arrays
cannot. `Var[]` is an array whose individual elements are `Var`, not a `Var`
containing an array. Its default value is `None`.
[Sources: variables](https://fallout.wiki/wiki/Resource:Creation_Kit/Variable_Reference#Var_Variables),
[migration guide](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4#Var_Type),
[casts](https://fallout.wiki/wiki/Resource:Creation_Kit/Cast_Reference).

```papyrus
Function UsePayload()
  Var payload = 6
  if payload is Int
    Int count = payload as Int
  endif
  Var[] arguments = new Var[2]
  arguments[0] = 6
  arguments[1] = "ready"
  ; Var invalid = arguments ; invalid: arrays cannot be boxed in Var
EndFunction
```

### `Is` and explicit casts

```text
cast      ::= expression 'As' type
type-test ::= expression 'Is' type
```

`Is` tests primitive types strictly: a `Float` value is not an `Int` even if a
numeric cast succeeds. Object tests are looser and account for compatible script
types. `Is` returns a Boolean; it does not change the expression's static type or
perform the later cast for the programmer. CK recommends a single object cast
and `None` check when the converted object will be used anyway.
[Sources: Cast Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Cast_Reference),
[Is migration note](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4#Is_Operator).

The Fallout 4 cast reference adds these rules to the shared baseline:

| Destination | Fallout 4 rule |
| --- | --- |
| `Bool` | Struct is true if non-`None`; `Var` uses its contained value's conversion; array is true when length is at least one |
| `Int` | A boxed value converts as its contents would; incompatible contents produce `0` |
| `Float` | A boxed value converts as its contents would; incompatible contents produce `0.0` |
| `String` | Struct prints its member values; `Var` prints its contained value; collections may be truncated by an unspecified internal buffer |
| Script type | Boxed object can convert; incompatible contents produce `None` |
| Array type | Explicit conversion only; makes a new array and converts each element; failed elements receive the destination default |
| Struct type | CK states that nothing can be cast to a struct |
| `Var` | Implicit or explicit conversion from non-array values |

[Source: Fallout 4 Cast Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Cast_Reference).
The "nothing can be cast to a struct" statement conflicts with the general
description that structs can be boxed in `Var` and then used through casts.
Do not resolve that conflict by inventing an unboxing rule: target compiler/VM
validation is needed. Struct reference assignment remains distinct from casting.

### Special name parameters are not freely computed strings

`ScriptEventName`, `CustomEventName`, and `StructVarName` are special function
parameter categories. Each accepts a raw string literal, whose contents the
compiler checks against a declaration. The preceding argument's type supplies
the checked context; if no preceding argument exists, the receiver's type does.
They are not general storage types established by the reference, and a string
variable is rejected even if its value is constant or its current value names
an existing member.
[Source: Function Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Function_Reference#Special_Parameter_Types).

| Parameter category | Compiler checks literal against |
| --- | --- |
| `ScriptEventName` | Events on the relevant script type |
| `CustomEventName` | Custom events declared by the relevant script type |
| `StructVarName` | Members of the relevant struct type |

```papyrus
Function FindSample(Sample[] samples)
  Int position = samples.FindStruct("Weight", 1.0)
  String memberName = "Weight"
  ; position = samples.FindStruct(memberName, 1.0) ; invalid literal category
EndFunction
```

The language reference still lists Boolean, integer, float, string, and `None`
literals. Names used as special parameters are ordinary quoted tokens with
extra compile-time validation; the CK pages do not introduce a runtime
`Type` literal value. Qualified script types are type/name syntax, not a general
first-class type object.
[Source: Literals Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Literals_Reference).

## Fallout 4 properties, groups, flags, and native scripts

### Property groups

```text
group ::= 'Group' identifier flags* newline documentation?
          property+ 'EndGroup' newline
```

Groups organize properties for CK/game display; they do not add lexical scope
or require `GroupName.PropertyName` access. At least one property is required.
Group names are unique in a source file. A child group with a parent's name
merges with it; parent properties precede child properties. Group order and
within-group property order are preserved; ungrouped property order is not.
[Source: Group Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Group_Reference).

```papyrus
Group Settings CollapsedOnRef
  {Editor settings in source order.}
  Quest Property Controller Auto Const Mandatory
  Int Property Threshold = 3 Auto
EndGroup
```

### Flags and legal declaration sites

Use the target's actual `.flg` file for editor/user flags; keyword modifiers and
user-defined flags are different mechanisms. CK flags have these documented
sites and effects. `Mandatory` is an editor warning condition, not a guarantee
that a runtime property is non-`None`.
[Sources: Flag Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Flag_Reference),
[compiler flags-file input](https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus_Compiler#Flags).

| Flag/modifier | Legal site described by CK | Effect/constraint |
| --- | --- | --- |
| `Conditional` | Script, script variable, auto property | Exposes condition-system data; variable/property requires a conditional script; only one conditional script attachable to an object |
| `Hidden` | Script, property, struct member | Hides the corresponding editor selection/field; struct-member use is distinct from ordinary variable flags |
| `Default` | Script | CK script-picker classification |
| `Mandatory` | Property | Warns/red-marks missing editor values |
| `CollapsedOnRef` | Group | Initially collapsed in reference property window |
| `CollapsedOnBase` | Group | Initially collapsed in base-object property window |
| `Collapsed` | Group | Initially collapsed in both contexts |
| `Const` | Script, non-struct variable, auto property | Restricts storage/writes; save-game initialization rules differ by site |
| `Native` | Script, engine function/event declaration | Engine-defined script/type or callable; native function requires native script |
| `Global` | Function | No instance `Self`; forbidden on events |
| `DebugOnly` | Script, function | Call sites omitted with release compilation |
| `BetaOnly` | Script, function | Call sites omitted with final compilation |

The CK property page distinguishes three property forms as in the shared
baseline. `Const` applies to auto properties; it is not interchangeable with
`AutoReadOnly`. An auto const property is editor-filled, may retain a declared
literal fallback, and cannot be assigned in script. An auto read-only property
requires its initializer in source.
[Source: Property Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Property_Reference).

### Modern flags-file scopes and combinations

The published `Institute_Papyrus_Flags.flg` outline adds `StructVar` and `Group`
allowed-site names to the flags-file mechanism. These scopes distinguish a
struct member from an ordinary script variable and a group from its properties.
The outline assigns `Hidden` bit 0 to scripts/properties/struct members,
`Conditional` bit 1 to scripts/variables, `Default` bit 2 to scripts,
`CollapsedOnRef` bit 3 and `CollapsedOnBase` bit 4 to groups, and `Mandatory`
bit 5 to properties. These named flags are supplied by configuration; they
are not reserved Papyrus keywords. The real selected SDK flags file remains
authoritative if it differs from this published outline.
[Source: Open Papyrus flags-file transcription](https://open-papyrus.github.io/docs/Papyrus_Language_Reference/Lexial_structure/Flags.html).

The outline defines a composite as `Flag Collapsed CollapsedOnRef & CollapsedOnBase`.
This makes `Collapsed` apply both component flags; `&` in this declaration is a
flags-file combinator, not the Papyrus logical `&&` expression operator. A
small original example of the same form is:

```text
Flag CompactReference 3 { Group }
Flag CompactBase 4 { Group }
Flag Compact CompactReference & CompactBase
```

The reconstructed flags parser accepts numeric definitions with optional
allowed-site blocks, and a composite definition with two or more named
components. It recognizes `Script`, `Property`, `Variable`, `StructVar`,
`Function`, and `Group` scopes. A parser production does not establish whether
forward component references, conflicting scopes, cycles, duplicate bit
indices, or all possible numeric indices pass the target compiler's validation.
The outline only establishes the used indices above; the common flags metadata
must not be conflated with Starfield's guard/access keyword modifier encoding.
[Source: reconstructed FlagsParser](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/FlagsParser.g4).

### Const storage and persistence rules

A const script cannot contain states or mutable auto properties/variables;
the runtime may discard its instance. Its reference cannot be stored in
object-level script variables. Const does not mean that its functions have no
effects: fragments can call other objects. A const variable requires its
initializer on its declaration line and cannot later be assigned. Const struct
members are forbidden. A const auto property's value comes from the editor;
saved property values are ignored so changed plugin values take effect when
loading a save.
[Sources: flags](https://fallout.wiki/wiki/Resource:Creation_Kit/Flag_Reference),
[const migration sections](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4).

```papyrus
Int Capacity = 12 Const
Quest Property Destination Auto Const

Function InvalidWrites()
  ; Capacity = 13 ; invalid write to Const variable
  ; Destination = None ; invalid write to Const auto property
EndFunction
```

CK pages disagree in wording about whether native/const scripts can contain
const variables: the script-structure page says no variables, whereas the flag,
variable, and migration descriptions permit const variables in const scripts
and the variable page bans only non-const variables in native/const scripts.
Preserve that distinction as an unresolved compiler acceptance question rather
than applying the broadest prohibition to every context.
[Sources: script header](https://fallout.wiki/wiki/Resource:Creation_Kit/Script_File_Structure#Header_Line),
[variables](https://fallout.wiki/wiki/Resource:Creation_Kit/Variable_Reference),
[flags](https://fallout.wiki/wiki/Resource:Creation_Kit/Flag_Reference).

### Native script boundary and event declarations

Fallout 4 makes `Native` a script-level marker. Native scripts describe types
known to the engine; they cannot contain states or auto properties. Only native
scripts may define new ordinary engine events or native functions. Non-native
scripts handle inherited events and use `CustomEvent` for new script-sent
notifications. Merely marking a function native does not install a binding:
the game reports a missing engine implementation when called.
[Sources: migration guide](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4#Native_Scripts),
[Function Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Function_Reference),
[Events Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Events_Reference).

### Release/final code generation

The Fallout 4 compiler's `-release`/`-r` strips calls to `DebugOnly` functions;
`-final` strips calls to `BetaOnly` functions. Script-level flags apply to its
functions when used from other scripts. CK's release-final mode combines release,
optimization, and final options; the default editor compilation includes these
calls. This is code generation, not a runtime logging switch.
[Sources: compiler options](https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus_Compiler),
[conditional compilation](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4#Conditionally-Compiled_Script_Functions).

Keep required program effects out of arguments to a removable call. The CK
summary does not specify all argument-evaluation, returned-value, or side-effect
cases after removal. The reconstructed release processor is supplementary
evidence for rewriting call nodes, not sufficient proof of every compiler mode.
[Source: reconstructed release processor](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusReleaseProcessorFO4.g).

## Fallout 4 arrays and compiler-provided operations

`New T[expression]` accepts an integer variable/expression, unlike Skyrim's
literal-size form. Creation is restricted to executable bodies. Element types
cannot themselves be arrays, so multidimensional array types are rejected.
Fresh scalar slots contain their default values; object/struct slots contain
`None`. A `None` array's `Length` is documented to return `0`.
[Source: Array Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Array_Reference).

```papyrus
Function CreateSamples(Int count)
  Sample[] samples = new Sample[count]
  if count > 0
    samples[0] = new Sample
  endif
EndFunction
```

The following signatures use `T` for the receiver array's element type and `M`
for the selected struct member's type. They are explanatory metavariables, not
generic syntax accepted in a `.psc` file. The special-name parameter must be a
raw literal and is checked against the receiver struct type.

| Signature | Effect and documented result | Source |
| --- | --- | --- |
| `Int Function Find(T akElement, Int aiStartIndex = 0) Native` | Search forward; returns matching index or negative when absent | [Find](https://fallout.wiki/wiki/Resource:Creation_Kit/Find_-_Array) |
| `Int Function RFind(T akElement, Int aiStartIndex = -1) Native` | Search backward; `-1` starts at end; negative when absent | [RFind](https://fallout.wiki/wiki/Resource:Creation_Kit/RFind_-_Array) |
| `Int Function FindStruct(StructVarName asVarName, M akElement, Int aiStartIndex = 0) Native` | Search matching member value forward; matching struct index or negative | [FindStruct](https://fallout.wiki/wiki/Resource:Creation_Kit/FindStruct_-_Array) |
| `Int Function RFindStruct(StructVarName asVarName, M akElement, Int aiStartIndex = -1) Native` | Search member value backward; matching struct index or negative | [RFindStruct](https://fallout.wiki/wiki/Resource:Creation_Kit/RFindStruct_-_Array) |
| `Function Add(T akElement, Int aiCount = 1) Native` | Append one or repeated elements, growing array; no return | [Add](https://fallout.wiki/wiki/Resource:Creation_Kit/Add_-_Array) |
| `Function Insert(T akElement, Int aiLocation) Native` | Insert and grow at location; no return | [Insert](https://fallout.wiki/wiki/Resource:Creation_Kit/Insert_-_Array) |
| `Function Remove(Int aiLocation, Int aiCount = 1) Native` | Remove consecutive elements and shrink; no return | [Remove](https://fallout.wiki/wiki/Resource:Creation_Kit/Remove_-_Array) |
| `Function RemoveLast() Native` | Remove final element and shrink; no return | [RemoveLast](https://fallout.wiki/wiki/Resource:Creation_Kit/RemoveLast_-_Array) |
| `Function Clear() Native` | Remove all elements, leaving a zero-length array; no return | [Clear](https://fallout.wiki/wiki/Resource:Creation_Kit/Clear_-_Array) |

`Add(structValue, count)` repeats the same struct reference, not distinct copies.
Array aliasing therefore matters both for resizing and member mutation. `Clear`
produces an allocated empty array; do not equate this with assigning `None`.
The individual method pages do not comprehensively specify invalid counts,
out-of-range insert/remove positions, calls on `None`, capacity overflow, or
search through `None` struct entries. Those failure cases remain runtime QA
requirements rather than invented clamping or exception rules.
[Sources: Add note](https://fallout.wiki/wiki/Resource:Creation_Kit/Add_-_Array),
[Clear](https://fallout.wiki/wiki/Resource:Creation_Kit/Clear_-_Array).

The CK `RFindStruct` page's example accidentally calls `RFind` and advances the
start index. Follow its signature and backward-search description, not that
example. After a match at index `i > 0`, continue at `i - 1`; stop at zero,
because passing `-1` restarts at the end.
[Sources: RFindStruct](https://fallout.wiki/wiki/Resource:Creation_Kit/RFindStruct_-_Array),
[RFind](https://fallout.wiki/wiki/Resource:Creation_Kit/RFind_-_Array).

The migration page says the array size restriction remains enforced but does
not name a numeric bound. Do not carry Skyrim's `128` bound into a future target
as an authoritative Fallout 4/Starfield limit based only on that sentence.
Distinguish the compiler's constant-size validation, the VM's allocation limit,
and PEX operand/count layout; each needs versioned evidence.
[Source: Array Sizes](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4#Array_Sizes).

## Fallout 4 custom and remote events

### Definitions and handler signatures

```text
custom-declaration ::= 'CustomEvent' identifier newline
remote-handler     ::= 'Event' defining-script '.' event-name
                       '(' defining-script sender [',' original-parameters] ')'
                       newline statements 'EndEvent' newline
custom-handler     ::= 'Event' defining-script '.' custom-name
                       '(' defining-script sender ',' 'Var[]' arguments ')'
                       newline statements 'EndEvent' newline
```

A custom declaration is at script scope, outside functions/events/properties,
and cannot conflict with a function/event in the script or parent. Handler
qualification and sender type must identify the original declaring script.
For inherited ordinary events, use the least-derived type that actually declares
the event, not an arbitrary receiver subtype. Remaining remote parameters match
the original event; custom handlers have exactly sender plus `Var[]`.
[Source: Events Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Events_Reference).

```papyrus
; Producer script, named Example:Producer and derived from Quest
CustomEvent Ready

Function NotifyReady()
  Var[] arguments = new Var[1]
  arguments[0] = 3
  SendCustomEvent("Ready", arguments)
EndFunction
```

```papyrus
; Consumer script, derived from Quest
Example:Producer Property Source Auto Const Mandatory

Event OnInit()
  RegisterForCustomEvent(Source, "Ready")
EndEvent

Event Example:Producer.Ready(Example:Producer akSender, Var[] akArgs)
  if akArgs.Length == 1 && akArgs[0] is Int
    Int amount = akArgs[0] as Int
  endif
EndEvent
```

These are invalid: a sender typed as a parent instead of the named declaring
type; an `Int` parameter instead of `Var[]`; an undeclared custom event name;
a string variable in the registration's name slot; direct invocation of a
custom/remote handler. An ordinary event's direct call also does not relay it to
remote subscribers.
[Sources: Events Reference](https://fallout.wiki/wiki/Resource:Creation_Kit/Events_Reference),
[Custom Papyrus Events](https://fallout.wiki/wiki/Resource:Creation_Kit/Custom_Papyrus_Events).

### Registration and send methods

These are instance methods on the native `ScriptObject` base, not standalone
syntax operations. The Fallout 4 CK pages specify no return value:

| Exact CK signature | Role | Source |
| --- | --- | --- |
| `Function RegisterForCustomEvent(ScriptObject akSender, CustomEventName asEventName) Native` | Subscribe this script instance to sender's declared custom event | [RegisterForCustomEvent](https://fallout.wiki/wiki/Resource:Creation_Kit/RegisterForCustomEvent_-_ScriptObject) |
| `Function UnregisterForCustomEvent(ScriptObject akEventSource, CustomEventName asEventName) Native` | Remove this instance's matching subscription | [UnregisterForCustomEvent](https://fallout.wiki/wiki/Resource:Creation_Kit/UnregisterForCustomEvent_-_ScriptObject) |
| `Function SendCustomEvent(CustomEventName asEventName, Var[] akArgs = None) Native` | Send receiver script's own/inherited custom event, without waiting | [SendCustomEvent](https://fallout.wiki/wiki/Resource:Creation_Kit/SendCustomEvent_-_ScriptObject) |
| `Function RegisterForRemoteEvent(ScriptObject akEventSource, ScriptEventName asEventName) Native` | Subscribe this instance to engine events received by source | [RegisterForRemoteEvent](https://fallout.wiki/wiki/Resource:Creation_Kit/RegisterForRemoteEvent_-_ScriptObject) |
| `Function UnregisterForRemoteEvent(ScriptObject akEventSource, ScriptEventName asEventName) Native` | Remove this instance's matching subscription | [UnregisterForRemoteEvent](https://fallout.wiki/wiki/Resource:Creation_Kit/UnregisterForRemoteEvent_-_ScriptObject) |

Subscriptions belong to an individual script instance, not every script attached
to a form/alias/effect. Sending returns immediately; it does not wait for receivers
or establish delivery order. The sender supplied to handlers is the object on
which `SendCustomEvent` was called. CK describes automatic unregistration when a
quest stops (including its aliases) and when an active effect is removed; the
overview instead says quest restarts. Treat exact restart/stop timing as an
engine lifecycle verification item. Missing sender, duplicate subscription,
delivery after unregistration, and payload mutation during concurrent delivery
are not comprehensively defined by these pages.
[Sources: custom event overview](https://fallout.wiki/wiki/Resource:Creation_Kit/Custom_Papyrus_Events),
[registration](https://fallout.wiki/wiki/Resource:Creation_Kit/RegisterForCustomEvent_-_ScriptObject),
[sending](https://fallout.wiki/wiki/Resource:Creation_Kit/SendCustomEvent_-_ScriptObject).

### `ScriptObject` and event-model migration

Fallout 4's common `ScriptObject` represents a script instance independently of
its bound game object. Its functions can run when that game object is unavailable;
functions from a more specific native type may still fail. Timer registration
replaces Skyrim update events; hit/magic-effect events require registrations and
inventory events require filters. These are declaration and runtime differences,
not keywords, and a compiler should not assume Skyrim event declarations when
checking a Fallout 4 project.
[Sources: migration guide](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4),
[ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/ScriptObject_Script).

Fallout 4 changes state callback signatures to
`Event OnBeginState(String asOldState)` and
`Event OnEndState(String asNewState)`; use target declarations rather than the
Skyrim zero-argument callback shapes. Runtime transition details belong in
[Runtime semantics](runtime.md).
[Source: ScriptObject event declarations](https://fallout.wiki/wiki/Resource:Creation_Kit/ScriptObject_Script).

### Dynamic calls and properties

Fallout 4 adds dynamic communication methods that deliberately omit a static
dependency on the named script/member. Their names are ordinary `String`
parameters, unlike `ScriptEventName`/`CustomEventName`/`StructVarName` categories.
The compiler cannot validate the dynamic name, signature, or argument conversions.
The caller must supply exact parameter/property types, including explicit
upcasts from an `Actor` to an expected `ObjectReference` or `Form`.
[Source: Inter-mod Communication](https://fallout.wiki/wiki/Resource:Creation_Kit/Inter-mod_Communication).

| CK signature | Documented result/failure | Source |
| --- | --- | --- |
| `ScriptObject Function CastAs(String asScriptName) Native` | Returns selected script or `None` if missing/not attached; selection among multiple eligible copies is unspecified | [CastAs](https://fallout.wiki/wiki/Resource:Creation_Kit/CastAs_-_ScriptObject) |
| `Var Function CallFunction(String asFuncName, Var[] aParams) Native` | Synchronous result; missing function errors; exact argument types required | [CallFunction](https://fallout.wiki/wiki/Resource:Creation_Kit/CallFunction_-_ScriptObject) |
| `Function CallFunctionNoWait(String asFuncName, Var[] aParams) Native` | Immediate return without result; same name/type checks deferred to runtime | [CallFunctionNoWait](https://fallout.wiki/wiki/Resource:Creation_Kit/CallFunctionNoWait_-_ScriptObject) |
| `Var Function GetPropertyValue(String asPropertyName) Native` | Returns value; missing property errors | [GetPropertyValue](https://fallout.wiki/wiki/Resource:Creation_Kit/GetPropertyValue_-_ScriptObject) |
| `Function SetPropertyValue(String asProperyName, Var aValue) Native` | Synchronous set; missing property/wrong value type errors | [SetPropertyValue](https://fallout.wiki/wiki/Resource:Creation_Kit/SetPropertyValue_-_ScriptObject) |
| `Function SetPropertyValueNoWait(String asProperyName, Var aValue) Native` | Asynchronous set without result; same exact-type requirement | [SetPropertyValueNoWait](https://fallout.wiki/wiki/Resource:Creation_Kit/SetPropertyValueNoWait_-_ScriptObject) |
| `Var Function CallGlobalFunction(String asScriptName, String asFuncName, Var[] aParams) Native Global` | Synchronous result; missing script/function errors; exact types required | [CallGlobalFunction](https://fallout.wiki/wiki/Resource:Creation_Kit/CallGlobalFunction_-_Utility) |
| `Function CallGlobalFunctionNoWait(String asScriptName, String asFuncName, Var[] aParams) Native Global` | Immediate return without result; runtime lookup/type failures | [CallGlobalFunctionNoWait](https://fallout.wiki/wiki/Resource:Creation_Kit/CallGlobalFunctionNoWait_-_Utility) |

The CK setter signatures spell `asProperyName` without the second `t`, while
their parameter prose uses `asPropertyName`; retain the recorded signature
without claiming that named-argument spelling is verified against SDK source.
None of these signatures specifies an optional argument/default. `CastAs`
can cross between scripts attached to the same form without manually walking
their inheritance tree, unlike the static `As` syntax. These methods do not
turn custom-event literal-name checks into dynamic string lookups.
[Sources: CastAs](https://fallout.wiki/wiki/Resource:Creation_Kit/CastAs_-_ScriptObject),
[SetPropertyValue](https://fallout.wiki/wiki/Resource:Creation_Kit/SetPropertyValue_-_ScriptObject).

## Starfield additions and the evidence boundary

The following sections identify known additions without promoting incomplete
reconstructions into a definitive Bethesda standard. Source positions shown by
a parser are candidate syntactic positions; legal flag combinations and VM
effects still need the actual target compiler/declarations.
[Sources: versioned syntax inventory](https://starfieldwiki.net/wiki/Starfield_Mod:Papyrus_Syntax_Reference),
[pinned compiler reconstruction](https://github.com/fireundubh/OpenPapyrus/tree/39065b24e61a66c070f20d86c5f669490cfa6ef1).

### Guard declarations and blocking lock blocks

The reconstructed parser places a named guard at top level, alongside fields,
structs, groups, functions, and states. It places `LockGuard` blocks inside
executable statement lists. This small candidate example agrees with its parser
and generator descriptions; it is not claimed to have been compiled in CK:

```papyrus
ScriptName Example:ProtectedCounter
Guard CounterGuard
Int Counter = 0

Function Increment()
  LockGuard CounterGuard
    Counter += 1
  EndLockGuard
EndFunction
```

Candidate notation:

```text
guard-declaration ::= 'Guard' identifier flags* newline
lock-block        ::= 'LockGuard' guard-list newline
                      statements 'EndLockGuard' newline
guard-list        ::= identifier (',' identifier)*
```

[Sources: reconstructed parser](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusParserSF1.g4),
[reconstructed generator](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusGenSF1.g).
The parser also accepts a parenthesized guard list, but that alternative has not
been verified with Bethesda's compiler. Guard names are declarations, not `Bool`
variables or arbitrary lock expressions.

The reconstructed type walker describes uniqueness/member-name conflict checks,
rejecting locks in global functions, tracking locked guards per scope, and
checking guard use across state overrides. The generator describes reverse-order
unlock and unlock on scope exit. Those comments are useful investigation leads;
they do not establish VM reentrancy, implicit/default guards, lock ordering,
latent-call behavior, deadlock resolution, or exception unwinding.
[Sources: reconstructed type walker](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusTypeWalkerSF1.g),
[generator](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusGenSF1.g).

### Try-lock syntax is unresolved

The `4.7.0.5` inventory recognizes `TryLockGuard`, `ElseTryLockGuard`, and
`EndTryLockGuard`. It does not provide a complete production. The pinned
OpenPapyrus files disagree:

| Evidence | Candidate shape |
| --- | --- |
| `PapyrusParserSF1.g4` | `TryLockGuard ID`, optional `Else`, then `EndTryLockGuard` |
| `PapyrusTypeWalkerSF1.g` | Try node has one or more IDs, zero or more `ElseTryLockGuard` branches, optional `Else` |
| `PapyrusGenSF1.g` | Generator comment shows `TryLockGuard ResultVar Guard1`; branch comment has result variable and guard; tree rule has `ID ID+` |
| `PapyrusLexerSF1.g4` | Recognizes `ElseTryLockGuard` although the reconstructed parser does not consume it |

[Sources: inventory](https://starfieldwiki.net/wiki/Starfield_Mod:Papyrus_Syntax_Reference),
[parser](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusParserSF1.g4),
[walker](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusTypeWalkerSF1.g),
[generator](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusGenSF1.g).

In particular, the generator may expose temporary result identifiers used after
lowering. Do not infer from that alone that source requires a user-declared
Boolean result variable. No try-lock production in this guide is an accepted
source contract until versioned Bethesda-compiler validation resolves the
conflict. `TryGuard`/`EndGuard` are experimental Caprica spellings, not the
Starfield inventory's keyword spellings.
[Source: Caprica parser](https://github.com/Orvid/Caprica/blob/e4dee0860914d75e770d3f9ab374f7aba474b701/Caprica/papyrus/parser/PapyrusParser.cpp).

### Guard and access modifiers

Starfield's inventory adds `RequiresGuard`, `ProtectsFunctionLogic`, `SelfOnly`,
`Private`, `Protected`, and `Internal`. Its descriptions alone do not establish
legal sites or access checks. The reconstructed parser's broad user-flag rule
permits the tokens wherever `userFlags` occurs; a permissive parser production
is not proof that every placement passes semantic validation.
[Sources: inventory](https://starfieldwiki.net/wiki/Starfield_Mod:Papyrus_Syntax_Reference),
[parser](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusParserSF1.g4).

| Addition | Located evidence | Still unresolved |
| --- | --- | --- |
| `RequiresGuard(g1, g2)` | Parser accepts parenthesized guard identifiers; walker mentions functions/property accessors | Complete legal sites, inherited contract, checks on fields/properties/calls, satisfaction by implicit guard |
| `ProtectsFunctionLogic` | Parser comment associates it with guard definitions | Exact protected operations and legal declaration combinations |
| `SelfOnly` | Lexer/parser and inventory recognize token | Difference from private visibility, applicability to variables/properties/functions, effect on parent access |
| `Private` | Lexer/parser token | Defining-script access, inheritance/state/accessor restrictions |
| `Protected` | Lexer/parser token | Derived-script access and instance restrictions |
| `Internal` | Lexer/parser token | Definition of assembly/package/namespace boundary and legal positions |

The familiar meanings of these words in C# are not sufficient evidence for
Papyrus. They must not be implemented as aliases for guessed access modes.
The walker does mention access checks during function resolution, but does not
provide a complete authoritative visibility table.
[Source: reconstructed semantic walker](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusTypeWalkerSF1.g).

### Struct matching, special types, and grammar incompleteness

The reconstructed Starfield walker recognizes an `ARRAYGETALLMATCHINGSTRUCTS`
node; Caprica resolves an array member named `GetMatchingStructs` and explicitly
emits an experimental-syntax warning. This establishes a feature to investigate,
not its exact source signature, optional arguments, result ownership, matching
semantics, or failure behavior. Do not invent a default start/count value or
infer a public name directly from the bytecode node name.
[Sources: walker](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusTypeWalkerSF1.g),
[Caprica resolver](https://github.com/Orvid/Caprica/blob/e4dee0860914d75e770d3f9ab374f7aba474b701/Caprica/papyrus/PapyrusResolutionContext.cpp).

The reconstructed Starfield lexer contains `DependentType` and `Void`, but its
parser does not include them in the general type production. Its `Var` base-type
alternative is commented out, despite `Var[]` in indexed game declarations.
Those discrepancies prevent treating this grammar as a complete parseable
specification. `DependentType` is not established here as a user-declarable type
or literal; `Void` is not established as a required spelling of no-return
functions. Keep all such candidates in target-specific investigation scope.
[Sources: lexer](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusLexerSF1.g4),
[parser](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusParserSF1.g4),
[ScriptObject declaration index](https://papyrus.bellcube.dev/starfield/script/scriptobject/).

The parser also appears to allow properties/groups inside states and initialized
const locals. These are source-position leads only, because this reconstruction
contains the contradictions above. Do not silently generalize Fallout 4 legal
positions or infer target support from a lexer/parser token inventory.
[Source: reconstructed parser](https://github.com/fireundubh/OpenPapyrus/blob/39065b24e61a66c070f20d86c5f669490cfa6ef1/PapyrusParserSF1.g4).

### Native APIs and callback differences

The Starfield declaration index still shows `ScriptObject`, `Var[]` payloads,
per-script registrations, and string arguments to state callbacks. It renders
`RegisterForRemoteEvent` with a Boolean return, whereas the archived Fallout 4
page shows no return. Its event-name parameters render as `String`, whereas
Fallout 4's CK signature uses special-name parameter categories. This is a
declaration extraction/version issue to verify from the game's original `.psc`,
not evidence that literal validation disappeared. Do not copy a Fallout 4 API
signature unchanged into a Starfield declaration package.
[Source: Starfield ScriptObject index](https://papyrus.bellcube.dev/starfield/script/scriptobject/).

## Fallout 76 and other games

The upstream [Champollion decompiler](https://github.com/Orvid/Champollion/tree/bc961a0bdfb4831f8240e6dacee0818b4bf81e00)
names Fallout 76 as a supported PEX family. This is evidence of an artifact
consumer's scope, not a Bethesda source-language reference. No complete public,
versioned Fallout 76 CK language specification was established in this audit.
Do not infer its source grammar, `Var`/struct limits, const behavior, native
bindings, event lifecycle, or multiplayer execution model from Fallout 4.

If Fallout 76 becomes a target, obtain auditable game/compiler evidence for
every matrix cell, identify the game/PEX revision, and separately establish
source access and distribution terms. Artifact inspection can verify a PEX
header/opcode without proving the original source spelling or VM semantics.
[Sources: Champollion](https://github.com/Orvid/Champollion),
[Open Papyrus resource inventory](https://open-papyrus.github.io/docs/Additional_Resources.html).

Fallout 3 and Fallout: New Vegas use the earlier scripting family, not the
Papyrus dialect extension described in the Fallout 4 CK migration reference.
They are not implicit Papyrus targets. Future Bethesda games also require
their own evidence; "later engine" is not a compatibility specification.
[Source: CK comparison with previous scripting](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Previous_Scripting).

## Third-party compilers and experimental dialect extensions

Caprica deliberately has additional syntax such as `For`, `ForEach`, `Switch`,
and `Do`/`LoopWhile`; it must not define Bethesda conformance merely because it
accepts a script. Its pinned parser still marks `Guard`/`TryGuard` blocks as
experimental and terminates them with `EndGuard`. Such spellings must be kept
separate from Starfield CK's `LockGuard`/`TryLockGuard` inventory. Decompiler
output may choose a provisional spelling or reconstruct equivalent control flow;
it is not original source-language evidence.
[Sources: Caprica parser](https://github.com/Orvid/Caprica/blob/e4dee0860914d75e770d3f9ab374f7aba474b701/Caprica/papyrus/parser/PapyrusParser.cpp),
[Open Papyrus resources](https://open-papyrus.github.io/docs/Additional_Resources.html),
[Champollion](https://github.com/Orvid/Champollion).

## Versioned limits and implementation acceptance boundaries

| Domain | Established here | What must be established before support |
| --- | --- | --- |
| Fallout 4 compiler | CK identifies compiler banner as `2.X.X.X`; executable expressions allowed as array sizes; some size restriction retained | Exact build, numeric allocation bounds, constant/local const acceptance, release-call side effects |
| Fallout 4 VM | Resizable reference arrays, reference structs, per-script registration, asynchronous custom delivery documented | Failure behavior for bounds/None/capacity, struct unboxing, lifecycle edge cases |
| Fallout 4 PEX | CK explicitly rejects Skyrim/FO4 interchange by game ID | Exact supported header versions, layout/opcodes, counts and integer-width limits |
| Starfield compiler | Token inventory attributed to `PCompiler.dll 4.7.0.5` | Complete productions/semantics, try-lock syntax, access/guard contracts, flags file |
| Starfield VM | Public game declaration evidence plus reconstructed locking operations | Reentrancy, implicit locks, guard ownership/lifetime, latent calls, matching structs, limits |
| Starfield PEX | Maintainer codec/decompiler/reconstruction evidence | Versioned official artifact samples and exact layout/lock opcode validation |
| Fallout 76 | Maintainer decompiler recognizes family | All language/compiler/VM/PEX target contracts |

[Sources: Fallout 4 compiler](https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus_Compiler),
[migration](https://fallout.wiki/wiki/Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4),
[Starfield inventory](https://starfieldwiki.net/wiki/Starfield_Mod:Papyrus_Syntax_Reference),
[Champollion](https://github.com/Orvid/Champollion).
The active unresolved audit is tracked in [Roadmap](../planning/roadmap.md).
This guide's explicit unknowns are limits of source evidence; no engine/editor
QA or future-dialect implementation conformance is claimed.

## Source register

Accessed **2026-10-01**. The following revisions identify the exact mirror
material used; revision timestamps belong to the mirror and do not identify a
Bethesda compiler build. Original CK links are preserved by the archive pages.
The references and small signatures above are attributed; examples in this
guide are newly written.

| Source | Revision/version | Notes |
| --- | --- | --- |
| [FO4 language category](https://falloutck.uesp.net/w/index.php?title=Category:Papyrus_Language_Reference&oldid=1233) | `1233` | Inventory of 20 language-reference pages |
| [Default Value Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Default_Value_Reference&oldid=5682238) | `5682238`, 2026-08-17 | Added types' defaults |
| [Expression Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Expression_Reference&oldid=4994982) | `4994982`, 2024-10-06 | Stale grammar; dedicated feature pages take precedence |
| [Keyword Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Keyword_Reference&oldid=5682224) | `5682224`, 2026-08-17 | Reserved inventory |
| [Language Reference Notation](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Language_Reference_Notation&oldid=5682225) | `5682225`, 2026-08-17 | Notation conventions |
| [Literals Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Literals_Reference&oldid=5682253) | `5682253`, 2026-08-17 | Five literal kinds |
| [Operator Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Operator_Reference&oldid=5682227) | `5682227`, 2026-08-17 | Is precedence, comparison prose errors |
| [Papyrus Naming Conventions](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Papyrus_Naming_Conventions&oldid=4998794) | `4998794`, 2024-10-08 | Style only, explicitly copied Skyrim guidance |
| [State Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/State_Reference&oldid=5000650) | `5000650`, 2024-10-11 | Fallback prose error |
| [Statement Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Statement_Reference&oldid=5000652) | `5000652`, 2024-10-11 | Invalid/stale examples |
| [Identifier Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Identifier_Reference&oldid=5145849) | `5145849`, 2025-02-08 | Namespace naming |
| [Script File Structure](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Script_File_Structure&oldid=4999984) | `4999984`, 2024-10-10 | Imports and header restrictions |
| [Struct Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Struct_Reference&oldid=5000694) | `5000694`, 2024-10-11 | Member limits and reference semantics |
| [Variable Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Variable_Reference&oldid=5000714) | `5000714`, 2024-10-11 | Var and native/const wording |
| [Cast Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Cast_Reference&oldid=5682259) | `5682259`, 2026-08-17 | Maintained mirror; contradictions identified above |
| [Function Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Function_Reference&oldid=4995896) | `4995896`, 2024-10-07 | Special parameter categories |
| [Events Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Events_Reference&oldid=5001532) | `5001532`, 2024-10-11 | Handler syntax |
| [Property Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Property_Reference&oldid=4998906) | `4998906`, 2024-10-08 | Auto const behavior |
| [Group Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Group_Reference&oldid=5001140) | `5001140`, 2024-10-11 | Order and inheritance merge |
| [Flag Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Flag_Reference&oldid=4995796) | `4995796`, 2024-10-07 | Sites and flags |
| [Array Reference](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Array_Reference&oldid=4995239) | `4995239`, 2024-10-07 | Expression sizes and methods |
| [Add](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Add_-_Array&oldid=4995190) | `4995190`, 2024-10-07 | Repeated struct reference |
| [FindStruct](https://fallout.wiki/index.php?title=Resource:Creation_Kit/FindStruct_-_Array&oldid=5145973) | `5145973`, 2025-02-08 | Literal member-name restriction |
| [RFindStruct](https://fallout.wiki/index.php?title=Resource:Creation_Kit/RFindStruct_-_Array&oldid=4998938) | `4998938`, 2024-10-08 | Signature/example disagreement |
| [RegisterForCustomEvent](https://fallout.wiki/index.php?title=Resource:Creation_Kit/RegisterForCustomEvent_-_ScriptObject&oldid=5009810) | `5009810`, 2024-10-19 | Lifecycle notes |
| [SendCustomEvent](https://fallout.wiki/index.php?title=Resource:Creation_Kit/SendCustomEvent_-_ScriptObject&oldid=5010809) | `5010809`, 2024-10-19 | Payload and asynchronous send |
| [Differences from Skyrim](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Differences_from_Skyrim_to_Fallout_4&oldid=5145860) | `5145860`, 2025-02-08 | Migration coverage |
| [Papyrus Compiler](https://fallout.wiki/index.php?title=Resource:Creation_Kit/Papyrus_Compiler&oldid=5146243) | `5146243`, 2025-02-09 | Options and modes |
| [Starfield syntax inventory](https://starfieldwiki.net/w/index.php?title=Starfield_Mod:Papyrus_Syntax_Reference&oldid=99541) | `99541`, 2025-02-10; `PCompiler.dll 4.7.0.5` | Community extraction, explicitly incomplete |
| [OpenPapyrus](https://github.com/fireundubh/OpenPapyrus/tree/39065b24e61a66c070f20d86c5f669490cfa6ef1) | commit `39065b24e61a66c070f20d86c5f669490cfa6ef1` | Maintainer reconstruction, internally inconsistent |
| [Caprica](https://github.com/Orvid/Caprica/tree/e4dee0860914d75e770d3f9ab374f7aba474b701) | commit `e4dee0860914d75e770d3f9ab374f7aba474b701` | Experimental syntax is not CK standard |
| [Champollion](https://github.com/Orvid/Champollion/tree/bc961a0bdfb4831f8240e6dacee0818b4bf81e00) | commit `bc961a0bdfb4831f8240e6dacee0818b4bf81e00` | Artifact evidence only |
