# Papyrus engine semantics

This guide records engine behavior needed to reason about Papyrus programs and compiler changes.
Its reference baselines are the Skyrim Creation Kit Wiki and the archived Fallout 4 CK documentation,
with page revisions listed below.
It does not claim that Folio has implemented or verified every rule recorded here.
The [Papyrus guide](../papyrus.md) describes Folio's language boundary;
[dialects and targets](dialects.md) separates the language and runtime profiles of different games.

## Evidence and scope

Language acceptance, emitted instructions, engine scheduling, installed APIs, plugin data, and saved state are separate inputs.
A successful Folio check cannot prove that an object exists, that a native implementation is installed,
or that a running save can safely adopt a changed script.
Statements in this guide concern the Skyrim reference baseline unless explicitly qualified otherwise.

The Wiki combines reference descriptions, historical documentation, editor advice, and community observations.
An exact revision identifies the inspected text, not a verified game executable or compiler build.
Use these evidence categories when extending this guide:

| Category | Meaning |
| --- | --- |
| Reference requirement | A language or engine rule described by the CK reference, awaiting separate implementation evidence where applicable |
| Engine constraint | Behavior involving VM scheduling, attachment, native bindings, plugin data, or saves |
| Historical tool limitation | Behavior attributed to a particular compiler/editor, without assuming that Folio must reproduce it |
| Community observation | A warning, example, or reported defect that lacks a complete versioned reproduction |
| Unresolved | Conflicting sources or missing details; do not invent an executable rule |

The engine-method discussion covers language-wide operations and their interaction with execution.
It is not an inventory of CK, SKSE, or other script APIs.
Ordinary engine functions still need the declarations selected by the project.

## Script instances and execution chains

A script definition is a type; an attached script instance is an individual runtime object with its own state and data.
Multiple instances can use the same definition, and several scripts can be attached to one game object.
Those facts do not merge their variables or their state machines.
`Self` identifies the current script instance; it should not be treated as an interchangeable untyped game reference.

An event can arrive again before an earlier execution of that event finishes.
Each arrival can create another execution chain rather than replacing the previous one.
Calls within a chain execute in order, but the chain can suspend and other chains can make progress.
Changing state controls later dispatch; it does not cancel an existing execution chain.

### Instance locks and external calls

The Skyrim threading reference describes an implicit lock around each script instance.
Only one thread at a time executes against that locked instance.
Other threads wait until they can acquire it.
The reference does not guarantee first-in-first-out entry, fairness, or a deterministic interleaving.

A call is internal when its target shares the same `Self`, including an implementation in an ancestor script.
A call to another instance or a global function is external.
An external call can release the caller's instance lock while that call is in progress.
On returning, the caller may have to wait to reenter its original instance.
Another thread may have changed script data during that interval.

Native, scripted, latent, and non-delayed status do not replace this distinction.
Even a fast global native call can be external.
Conversely, the threading page warns that not every external call actually releases the lock long enough
for another thread to enter; programs should allow for reentry without requiring it to happen.

Do not equate a source line with an atomic execution unit.
A single expression can invoke accessors and functions, suspend, and resume with changed shared data.
The compiler must preserve the required evaluation order and captured values across those calls.
Scheduling details such as exact frame counts are not portable language guarantees.

### Properties and arrays

Reading or writing another instance's property is equivalent to an external accessor call.
The getter and setter can execute code, so their side effects matter even when property syntax looks like storage access.
The threading reference treats property operations on the current instance as internal.
It also describes optimizations for local properties, but that does not establish that every manual property is a plain field.

An external property increment can read a value, allow another chain to modify it, and then write a computed result.
Evaluating the receiver only once does not make that read/modify/write sequence atomic.
Local operations that remain under the same instance lock have a different boundary.
Adding an external call to an accessor can change the behavior of surrounding code.

Array element access, element writes, `Length`, `Find`, and `RFind` are described as operations rather than external calls.
This does not make an entire expression involving an array atomic:
its receiver, index, searched value, or right-hand side can itself make an external call.
Arrays are references, and aliases can observe the same elements.
A compiler optimization must preserve both aliasing and the values already obtained before a suspension point.

### Native, delayed, and latent behavior

| Term | Skyrim reference meaning | Consequence |
| --- | --- | --- |
| Native | Implemented by the engine or an installed native provider | A visible signature does not prove a usable runtime binding |
| Delayed native | A native operation that synchronizes with the game's frame processing | Latency and scheduling are engine concerns |
| Non-delayed native | A native operation that does not require that frame synchronization | Faster execution does not guarantee an instance remains locked |
| Latent | A call whose completion can suspend the calling execution chain | Subsequent caller code waits for completion |
| Scripted | Implemented in Papyrus | Not synchronized to frames merely because it is scripted |

These properties are not synonyms and are not inferred from a function's spelling.
The latent-function category explains that a zero-duration wait, early failure, or operation with no work
may complete without giving other threads an effective opportunity to enter.
Waiting is therefore not a guarantee that another particular event has run.

The older scripting comparison also describes requested delays as including later scheduling delay.
Do not promise exact wall-clock resumption times or use the reference's approximate throughput as a compiler limit.
Folio's declaration `native` fact alone does not encode latent or non-delayed classification.
No runtime compatibility claim follows from that metadata.

## States and dynamic dispatch

Each script instance has one current state.
Members outside explicit state blocks belong to the empty state.
An `Auto State` selects the initial state; initial-state selection must also account for inheritance.
Absence of a local auto state alone does not establish that the instance starts in the empty state.
The empty state is the fallback when no applicable auto state selects another initial state.
Multiple scripts attached to one game object can have separate current states.

A function or event in a named state must have a corresponding empty-state declaration
with the same return type and parameter types, including an applicable inherited declaration.
The runtime signature is not a fresh overload local to the state.
An empty overriding body suppresses behavior; it does not request automatic fallback.

### Dispatch and inheritance

The Skyrim state reference describes lookup in this order:

1. The current state's implementation in the receiving script.
2. The current state's implementation in its ancestors.
3. The empty-state implementation in the receiving script.
4. The empty-state implementation in its ancestors.

States inherited from parent scripts are merged with those declared by children.
An implementation in the child's matching state overrides the parent's implementation there.
A parent's state implementation can therefore take precedence over a child's empty-state implementation.
Do not flatten that order into a rule that derived-script code always wins.

Casting an object to a parent script type does not disable dynamic dispatch to its actual derived implementation.
`Parent` is the explicit mechanism for invoking an ancestor implementation on the current instance.
It is not a way to invoke a parent implementation on an arbitrary other receiver.
Properties are inherited but cannot be redefined as overrides in the child.
Private parent and child variables remain distinct even if their names match.

### GetState and GotoState

The language-wide instance signatures described by the Skyrim references are:

```papyrus
String Function GetState()
Function GotoState(String asNewState)
```

`GetState` obtains the current state as a string.
`GotoState` takes a state name matched without case sensitivity to state blocks.
An empty string selects the empty state, and the argument can be calculated at runtime.
Do not require every call argument to be a declared-state string literal.

The transition described by the dedicated `GotoState` page is sequential:

1. Execute the old state's `OnEndState` handling.
2. Change the current state.
3. Execute the new state's `OnBeginState` handling.
4. Return to the caller after both handlers finish.

The page explicitly says these two handlers do not overlap and the method waits for their completion.
The caller continues after the method returns; the transition does not act like `Return`.
This ordering does not settle nested transitions from within a handler or interactions with other suspended chains.
Those cases need identified engine evidence rather than a guessed recursive-transition model.

The dedicated `GetState` page labels that operation latent and says it is not executed asynchronously.
Treat this as a reference assertion requiring target verification, not as a reason to add a source-language `latent` flag.
The `GotoState` page says subsequent state reads return the supplied string exactly.
The Skyrim `States (Papyrus)` concept page and `GetState - All Scripts` page contain older
case-sensitive string-comparison claims that conflict with `Literals Reference`.
State-name matching, returned spelling, string interning, and string equality are different questions;
do not derive one from another or promise exact runtime casing until the conflicting evidence is resolved.

### OnBeginState and OnEndState

These events are available to all scripts and have no parameters:

```papyrus
Event OnBeginState()
Event OnEndState()
```

`OnEndState` belongs to the state being left; `OnBeginState` belongs to the state being entered.
Initializing an object directly into its auto state does not itself trigger `OnBeginState`.
The event page says a later transition to the same state still triggers `OnBeginState`.
It also describes transitions to undefined or invalidly named states, including the empty string.
Such transitions are not established compile-time errors merely because no explicit state block exists.
Handler lookup still depends on state and inheritance.

The references do not completely define failure, recursive transitions, or lock behavior inside lifecycle handlers.
Any Folio lowering of these methods must be audited against that distinction.
Emitting a call sequence alone does not verify the target engine's event delivery.

## Initialization and reset

`OnInit()` is described as running after script creation and property initialization.
A full property's setter can run before it to apply a masterfile/plugin value.
Auto properties use generated storage; manual property setters can perform arbitrary work.
Do not treat plugin initialization as identical to declaration literal initialization.

Until initialization finishes, the engine generally withholds other events and pauses other scripts
trying to call the instance or access its properties.
The `OnInit` reference gives exceptions for calls from another script's `OnInit`
and from a property setter applying masterfile data.
Initialization can therefore participate in cross-instance calls and should not be modeled as globally isolated execution.

| Object context | Initialization described by the Skyrim reference |
| --- | --- |
| Quest and alias | At game startup and again when quest startup resets them |
| Script running on a base object | At game startup when that script is first loaded |
| Persistent reference | At game startup when first loaded |
| Non-persistent reference | When the reference first loads |
| Reset object | After variables and properties are restored to initial values |

Quest startup and reset flags affect how often initialization occurs.
The reference specifically warns about quests without Run Once producing two initializations.
Cell reset can reinitialize references.
OnInit is therefore not a promise of exactly one invocation for the lifetime of a game object.
The page reports overlapping initialization restoring data underneath earlier executions.
Keep that warning separate from a universal guarantee about reset scheduling.

If `OnInit` is implemented in a named state, the page requires an empty-state declaration too.
The empty implementation can be empty, and the initial state need not be the empty state.
An existing object whose initialization already ran is not reinitialized simply because a save is loaded.
New variables should not rely solely on a newly added OnInit assignment for migration of existing saves.

## Saves, data, and script updates

The save system can record execution between statements or while a statement is in progress.
It retains execution state as well as script data.
An unchanged script can resume, but changed declarations and running functions require separate analysis.
These behaviors are engine update rules, not guarantees supplied by a compiler's successful build.

### Instance and member changes

| Change after a save | Behavior described by Save Files Notes |
| --- | --- |
| Add a script to an object | The new instance initializes from masterfile values and runs OnInit after loading |
| Remove an attachment | The existing saved script can remain attached; deleting its code can cause problems |
| Add a variable | Its declaration default, or the type default, initializes the new storage |
| Add an Auto property | A masterfile value can initialize it |
| Add a manual property | It does not automatically receive a masterfile value on the existing saved instance |
| Rename a variable/property | Treat as removing the old member and adding a new one |
| Change a member's type | Discard incompatible saved data and report a warning |
| Change a plugin property value | Do not generally overwrite a value already retained by the save |

Removed members become inaccessible to normal running code and their saved data is eventually discarded.
The source page marks manual querying of orphaned property data with a verification request;
that edge case remains unresolved here.
Changing default literals is not a save migration mechanism for existing storage.

A missing plugin can leave a Form reference pointing to a missing placeholder with ID zero rather than None.
Saving again can make that loss permanent even if the plugin returns later.
The page reports that different missing Form values can then compare equal.
This is a game-object observation, not permission to equate all invalid bindings with None.
API-specific FormList behavior is outside this guide's method scope.

### Running functions and saved code

Changes to a function matter especially when its execution was present in the save.
The reference describes retaining a saved old version until that execution completes,
while later calls use the current resource-file version.
This applies to changed code, local variables, parameters, return type, and removed functions.
An old frame can still try to call a removed or incompatibly changed member and fail.
Replacing a native implementation with a scripted implementation is a documented exception that discards the saved stack.

The source describes historical build/save prerequisites for this behavior.
It does not identify evidence for every Skyrim SE executable, compiler, or loading arrangement.
Do not assert that arbitrary hot replacement or save upgrades are supported by Folio.
Preserving PEX signatures does not by itself prove safe saved-stack compatibility.

## Reference lifetime and persistence

Persistence keeps a game reference loaded when it could otherwise unload.
It applies to actors and other object references and can consume engine resources.
It is separate from source visibility and from the ability to resolve a script type.

| Retaining condition | Reference lifetime described by the Skyrim page |
| --- | --- |
| Active function on an attached object | Retained until the function exits, including long latent execution |
| Reference assigned to an editor-filled property | Original reference can be permanently persistent |
| Any variable in a loaded script points to a reference | Temporarily retained while such references remain |
| Event registration on a reference | Retained until registration is removed, subject to other retainers |

Clearing an editor-filled property at runtime does not revoke the original permanent persistence marking.
Clearing variables or returning from a function can release temporary retention,
but unloading still depends on other references and engine systems.
Do not promise immediate destruction or unloading when a source variable is assigned None.
Arrays also have reference identity and aliases; their garbage collection is not a deterministic destructor facility.

## Runtime failures and diagnostics

Static type validity cannot prove object liveness, a native binding, plugin coherence, or valid array contents.
The Skyrim error reference distinguishes engine warnings from failures preventing proper execution.
Its examples are useful failure classes; they are not Folio diagnostic codes.

| Failure class | Engine boundary |
| --- | --- |
| Instance call on None | The engine aborts that call; a caller may subsequently receive None |
| Missing/incorrect native object binding | A script object exists but cannot service the native operation |
| Missing PEX or failed type loading | The requested implementation cannot load |
| Division or remainder by zero | A valid numeric expression can fail with a zero runtime operand |
| Out-of-range array index | Runtime length and index determine validity |
| Element access on a None array | No array storage exists for the access |
| Incompatible override or circular inheritance in loaded resources | Individually compiled files can disagree after deployment or updates |
| Attachment base-type mismatch | Plugin attachment does not fit the script's required native base |
| Property initialization mismatch | Missing, read-only, or differently typed properties can cause values to be skipped |
| Saved member/type no longer matches resources | The engine skips data or restores saved code and reports warnings |

The error page describes a failed call returning None and a secondary error when that value
is assigned into non-object storage.
It does not specify exact recovery values and continuation for every operation in the table.
Do not invent numeric defaults, automatic bounds clamping, or exception-like propagation.
Retain unresolved engine behavior rather than silently approximating it in lowering.

Runtime stack traces list the failing frame first and the engine entry event toward the end.
Native frames can have no PSC path or source line.
Line mappings require debug information to exist and the game to load it;
Folio's debug-info emission setting does not enable the game's own debug-information setting.
Logging, trace visibility, profiling, and VM budgets belong to engine INI configuration.
The historical INI defaults and performance claims are not target-independent recommendations.

## Compiler and attachment constraints

The reference compiler uses source import folders and a flag file.
Its documented duplicate-source policy selects the first occurrence in the import list.
Folio's project selection policy deliberately has its own ordering, documented in
[Projects and dependencies](../architecture/project-model.md#script-selection-and-runtime-requirements).
The CK command line, deployment advice, and single-file entry point are not Folio interfaces.

Compiler recovery can report cascaded errors after an earlier malformed construct.
The reported location can be where parsing failed rather than where the original mistake began.
The Compiler Errors page's explanation of EOF as an Event/Function abbreviation is not adopted here.
Its unversioned 38-character filename limit is a historical tool claim, not an established Papyrus language rule.

The array tutorial reports problematic indexed Find calls and unsupported compound element assignment.
Those warnings must be investigated as reference-compiler behavior; reproducing a historical miscompile
is not required language compatibility.
Likewise, its observation about reading a None array does not define a safe fallback value.

Attachment can expose several script instances deriving from the same base on one game object.
The extending/property tutorials warn that a cast to that base can select an unexpected instance
and that filled properties can behave unexpectedly with parent and child attachments.
These are engine/editor observations, not reasons to merge distinct instance storage in analysis.
The older scripting comparison also says only one Conditional script can attach to an object;
this needs attachment-level validation rather than a rule derivable from one source file.

## Fallout 4 runtime reference

The Fallout Wiki `Resource:Creation Kit` pages preserve Fallout 4 CK documentation.
They provide a separate reference baseline rather than requiring extrapolation from Skyrim.
The inspected pages still include inherited examples and inconsistent old links;
their revision IDs identify this archive's revisions, not the original CK publication revisions.
Matching descriptions establish documented agreement, not a tested binary compatibility claim.

| Area | Fallout 4 reference agreement or difference |
| --- | --- |
| Instance execution | Threading Notes describes serialized execution against a script and reentry during external or latent calls |
| External property compound assignment | Separate get/set can observe an intervening modification, as in the Skyrim discussion |
| State dispatch | States retains current-state-before-empty-state lookup, including parent implementations |
| State transitions | GotoState waits for both lifecycle handlers to finish without overlap and then resumes the caller |
| Same-state transition | GotoState explicitly says both lifecycle handlers still fire when the requested state is already current |
| Lifecycle signatures | OnBeginState receives the old state name; OnEndState receives the new state name |
| Initialization | OnInit remains parameterless; repeatable quest resets and reference creation timing are explicitly qualified |
| Saved data | Const variables/properties introduce exceptions to the usual precedence of saved values |
| Saved execution | Removed or modified active functions can finish using saved old code; native-to-scripted still discards the old stack |
| Persistence | Functions, editor-filled reference properties, variables, and event registrations retain references |
| Runtime failures | Common None/binding/array/arithmetic errors remain; struct-member mismatch and excessive stack depth are also documented |

### ScriptObject methods and lifecycle signatures

The Fallout 4 pages identify `ScriptObject` as the owner of the common methods and events.
This provides a target-specific declaration boundary; it does not imply that its complete API is intrinsic in Folio.
The common state methods retain their Skyrim-shaped signatures, but the lifecycle event signatures differ:

```papyrus
String Function GetState()
Function GotoState(String asNewState)
Event OnBeginState(String asOldState)
Event OnEndState(String asNewState)
Event OnInit()
```

The incoming-state handler receives the name of the state just left.
The outgoing-state handler receives the intended next state.
Do not reuse Skyrim's parameterless lifecycle-event validation for Fallout 4.
The dedicated GotoState page explicitly confirms waiting for old and new handlers,
and that transitioning to the already-current state still invokes both.
OnBeginState still does not run merely because an instance initializes into its auto state.

The archived GetState page omits Skyrim's latent note and case-sensitive comparison comment.
Omission is not evidence that GetState cannot suspend or that all spelling/caching questions are resolved.
Its stale link to an All Scripts page should not replace the dedicated ScriptObject method page as evidence.
Nested transitions and handler reentry still need engine-version-specific verification.

### Initialization, repeatable quests, and reference creation

The Fallout 4 OnInit page retains the property-initialization barrier and the exceptions for
other OnInit executions and masterfile property setters.
It explicitly warns that setting quest stages from OnInit, including indirectly called functions,
can deadlock; this is an engine control-flow hazard rather than a forbidden expression grammar.

Repeatable quest startup resets quests and aliases and can produce a second initialization after startup.
The reference limits that reset description to repeatable quests rather than importing Skyrim's Run Once wording.
Persistent references and base objects initialize at game startup.
Non-persistent references initialize when they first come into existence, explicitly not when their 3D loads.
Initialization, reference creation, cell loading, and 3D loading must therefore remain distinct concepts.
Variables and properties reset before the repeated OnInit, including reference reset on cell reset.

The page does not reproduce all Skyrim warnings about overlapping initialization.
Their absence is not proof that overlap is impossible in Fallout 4.
Only the documented agreement and differences above are established by this comparison.

### Timers and event lifetime

Fallout 4's ScriptObject timers provide targeted, one-shot callbacks rather than Skyrim-style recurring update registration.
They are declared native methods, not new Papyrus statements:

```papyrus
Function StartTimer(Float afInterval, Int aiTimerID = 0) Native
Function StartTimerGameTime(Float afInterval, Int aiTimerID = 0) Native
Function CancelTimer(Int aiTimerID = 0) Native
Function CancelTimerGameTime(Int aiTimerID = 0) Native
Event OnTimer(Int aiTimerID)
Event OnTimerGameTime(Int aiTimerID)
```

| Property | Real-time timer | Game-time timer |
| --- | --- | --- |
| Interval unit | Seconds | Game-time hours |
| Minimum described by the reference | No minimum specified | Values below 0.033 hours round up to that value, described as two minutes |
| Callback | OnTimer with the expired ID | OnTimerGameTime with the expired ID |
| Counting in menu mode | Paused | Paused |
| Automatic recurrence | None; start another timer for another callback | None; start another timer for another callback |

IDs belong to the receiving script; real-time and game-time timers use separate ID spaces.
Starting an already-counting timer with the same ID resets its interval rather than creating another pending timer.
Cancellation does nothing if that timer is absent or has already expired.
The pages do not promise to retract an already-queued event or terminate a callback already executing.
Resetting a countdown must likewise not be interpreted as canceling an old callback whose timer already expired.

Both callbacks are delivered to their originating script, not relayed to other scripts, aliases, or effects on the same form.
Quests and their aliases automatically cancel timers when the quest stops;
active magic effects cancel timers when removed.
The reference does not define whether this cancellation removes already-queued callbacks,
nor does it fully specify timer serialization and restoration across saves.

Real-time timers are affected by the game's global time modifier and are not an independent wall-clock facility.
During VATS playback, different world elements can have different time modifiers;
the OnTimer page says callback timing follows the player's modified speed.
Game-time callbacks can arrive after sleep, wait, fast travel, or jail time ends,
so the elapsed game-time interval can be much larger than the requested tick.
Both event pages say callbacks are not delivered during menu mode.

The StartTimer page also reports that ObjectReference scripts need an explicit ID argument
because the implicit default ID zero does not start a timer there.
This unversioned note conflicts with the ordinary default-argument presentation and lacks a reproduction;
retain it as a target/tool observation for QA, not a universal rejection of omitted timer IDs.
The source provides no corresponding claim about StartTimerGameTime or explicit zero values.

### Const data and saved aliases

Fallout 4's Save File Notes adds an important exception to saved-value precedence.
A changed const object variable receives its current script-initial value when loading.
A const property ignores its saved value and uses current masterfile data.
Normal variables that previously copied that property retain their own saved values.
For objects, structs, and arrays, repointing the property does not repoint aliases to the previous referenced value.
This is a reload/data rule; it does not require mutation of the previous object or array.

Const members still record values in the save even though loading ignores them.
Changing const to non-const allows those recorded values to take precedence again.
Changing non-const to const instead reasserts the current masterfile or script-initial value.
Do not model const as simply deleting a member from persistence.
Detailed const syntax and compile-time assignment constraints belong in [dialects and targets](dialects.md).

The archive also explicitly states that removing a mod and continuing with that save is unsupported.
Its removed-script and old-code descriptions explain what can happen, not an approved migration strategy.
A reference placed into a container while non-persistent can later fail to receive plugin reference-level changes,
because the resulting in-world reference is not linked back to the former one.
That observation concerns engine reference identity, not ordinary Papyrus assignment aliasing.

### Persistence, runtime failure, and evidence limits

The Fallout 4 persistence page retains the four retention classes described above.
It adds `DumpPapyrusPersistenceInfo` as a diagnostic for Papyrus retainers;
that command cannot explain retention by unrelated engine systems, such as an alias.
Several examples still use older event-registration names, so they are not an authoritative Fallout 4 API inventory.

The Runtime Errors page additionally documents mismatches in loaded struct member layouts and searches.
Two files compiled against different struct definitions can disagree even when each previously compiled successfully.
It also describes excessive call depth aborting a call and returning None,
without establishing a portable numeric recursion-depth limit.
An excessive-stack-count dump warns about workload or runaway scripts; it is not the same failure as deep recursion.

The Fallout 4 Threading Notes page is less detailed than the inspected Skyrim page.
It describes latent calls as potential opportunities for reentry, including calls on the same script's inherited APIs.
Do not mechanically impose Skyrim's simplified same-Self/internal-call formulation on every Fallout 4 latent call.
It does not independently specify all Skyrim notes about array operations, non-delayed globals, or unpredictable unlock exceptions.
Those gaps remain target-specific questions rather than new universal rules.
The Fallout 4 Compiler Errors page retains recovery/cascade advice but does not retain
the Skyrim page's EOF abbreviation explanation or filename-length claim.
Their omission does not prove a new maximum length; no Fallout 4 limit is inferred here.

## Starfield and unresolved verification

The Skyrim and Fallout 4 references establish no Starfield runtime profile.
Shared terminology, similar API signatures, or a known PEX format do not establish identical scheduling,
state-transition callbacks, initialization, persistence, saved-code restoration, or error recovery.
Modern syntax and target differences belong in [dialects and targets](dialects.md).

Before making a runtime compatibility claim for another game, identify its CK/SDK version,
engine version, relevant primary reference pages, and observed results for the rule concerned.
Guard syntax or new event signatures must not be backported into the Skyrim runtime description.
Lack of primary evidence is an explicit gap, not evidence that a feature is absent.

The outstanding implementation and external verification are tracked in the
[roadmap](../planning/roadmap.md), including state-string casing conflicts,
nested state transitions, OnInit/reset overlap, latent/non-delayed classification,
saved-stack compatibility, reference retention, and failure continuation.
Pure logic tests can verify compiler decisions but cannot establish actual VM behavior.

## Inspected Skyrim sources

These revisions record the inspected CK UESP material. Summaries above are original paraphrases.
Community examples and historical tool advice retain their evidence limits.

| Source | Revision |
| --- | --- |
| [Threading Notes (Papyrus)](https://ck.uesp.net/wiki/Threading_Notes_(Papyrus)) | 26058 |
| [Category:Latent Functions](https://ck.uesp.net/wiki/Category:Latent_Functions) | 5120 |
| [Category:Non-delayed Native Function](https://ck.uesp.net/wiki/Category:Non-delayed_Native_Function) | 5212 |
| [States (Papyrus)](https://ck.uesp.net/wiki/States_(Papyrus)) | 26037 |
| [GetState - All Scripts](https://ck.uesp.net/wiki/GetState_-_All_Scripts) | 25314 |
| [GotoState - All Scripts](https://ck.uesp.net/wiki/GotoState_-_All_Scripts) | 25357 |
| [OnBeginState](https://ck.uesp.net/wiki/OnBeginState) | 25592 |
| [OnEndState](https://ck.uesp.net/wiki/OnEndState) | 25609 |
| [OnInit](https://ck.uesp.net/wiki/OnInit) | 25616 |
| [Save Files Notes (Papyrus)](https://ck.uesp.net/wiki/Save_Files_Notes_(Papyrus)) | 25807 |
| [Persistence (Papyrus)](https://ck.uesp.net/wiki/Persistence_(Papyrus)) | 25710 |
| [Papyrus Runtime Errors](https://ck.uesp.net/wiki/Papyrus_Runtime_Errors) | 14218 |
| [Papyrus Compiler Reference](https://ck.uesp.net/wiki/Papyrus_Compiler_Reference) | 14147 |
| [Papyrus Compiler Errors](https://ck.uesp.net/wiki/Papyrus_Compiler_Errors) | 14138 |
| [Extending Scripts (Papyrus)](https://ck.uesp.net/wiki/Extending_Scripts_(Papyrus)) | 25013 |
| [Variables and Properties](https://ck.uesp.net/wiki/Variables_and_Properties) | 26132 |
| [Arrays (Papyrus)](https://ck.uesp.net/wiki/Arrays_(Papyrus)) | 24878 |
| [Differences from Previous Scripting](https://ck.uesp.net/wiki/Differences_from_Previous_Scripting) | 24974 |
| [INI Settings (Papyrus)](https://ck.uesp.net/wiki/INI_Settings_(Papyrus)) | 11982 |

Other inspected root pages add no independent engine contract here:
[Compiling Papyrus Scripts](https://ck.uesp.net/wiki/Compiling_Papyrus_Scripts) (8487) is incomplete;
[Fragments](https://ck.uesp.net/wiki/Fragments) (10238) is a navigation page;
[Console Commands (Papyrus)](https://ck.uesp.net/wiki/Console_Commands_(Papyrus)) (8687) lists diagnostic commands;
[Papyrus Glossary](https://ck.uesp.net/wiki/Papyrus_Glossary) (25703) provides introductory terminology.

## Inspected Fallout 4 sources

These are archived CK pages in Fallout Wiki's resource namespace.
The table preserves exact titles and archive revision IDs, independently of the Skyrim evidence.

| Source | Archive revision |
| --- | --- |
| [Resource:Creation Kit/Threading Notes (Papyrus)](https://fallout.wiki/wiki/Resource:Creation_Kit/Threading_Notes_(Papyrus)) | 5000856 |
| [Resource:Creation Kit/Persistence (Papyrus)](https://fallout.wiki/wiki/Resource:Creation_Kit/Persistence_(Papyrus)) | 4998824 |
| [Resource:Creation Kit/Save File Notes (Papyrus)](https://fallout.wiki/wiki/Resource:Creation_Kit/Save_File_Notes_(Papyrus)) | 4999666 |
| [Resource:Creation Kit/States (Papyrus)](https://fallout.wiki/wiki/Resource:Creation_Kit/States_(Papyrus)) | 5000654 |
| [Resource:Creation Kit/ScriptObject Script](https://fallout.wiki/wiki/Resource:Creation_Kit/ScriptObject_Script) | 4996704 |
| [Resource:Creation Kit/GetState - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/GetState_-_ScriptObject) | 5145947 |
| [Resource:Creation Kit/GotoState - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/GotoState_-_ScriptObject) | 5145977 |
| [Resource:Creation Kit/OnBeginState - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/OnBeginState_-_ScriptObject) | 4998736 |
| [Resource:Creation Kit/OnEndState - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/OnEndState_-_ScriptObject) | 4998979 |
| [Resource:Creation Kit/OnInit - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/OnInit_-_ScriptObject) | 4999019 |
| [Resource:Creation Kit/Papyrus Runtime Errors](https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus_Runtime_Errors) | 4998800 |
| [Resource:Creation Kit/Papyrus Compiler Errors](https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus_Compiler_Errors) | 4998786 |
| [Resource:Creation Kit/StartTimer - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/StartTimer_-_ScriptObject) | 5011938 |
| [Resource:Creation Kit/StartTimerGameTime - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/StartTimerGameTime_-_ScriptObject) | 5010076 |
| [Resource:Creation Kit/CancelTimer - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/CancelTimer_-_ScriptObject) | 5009852 |
| [Resource:Creation Kit/CancelTimerGameTime - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/CancelTimerGameTime_-_ScriptObject) | 5011631 |
| [Resource:Creation Kit/OnTimer - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/OnTimer_-_ScriptObject) | 5010693 |
| [Resource:Creation Kit/OnTimerGameTime - ScriptObject](https://fallout.wiki/wiki/Resource:Creation_Kit/OnTimerGameTime_-_ScriptObject) | 5010419 |
