//! Original summaries of Skyrim language constructs; reference revisions live in tooling.md.
use folio_papyrus::SyntaxKind;

pub(super) struct Entry {
    pub title: &'static str,
    pub description: &'static str,
    pub example: Option<&'static str>,
    pub reference_url: &'static str,
}

const LITERALS: &str = "https://ck.uesp.net/wiki/Literals_Reference";
const FUNCTIONS: &str = "https://ck.uesp.net/wiki/Function_Reference";
const EVENTS: &str = "https://ck.uesp.net/wiki/Events_Reference";
const PROPERTIES: &str = "https://ck.uesp.net/wiki/Property_Reference";
const STATES: &str = "https://ck.uesp.net/wiki/State_Reference";
const STATEMENTS: &str = "https://ck.uesp.net/wiki/Statement_Reference";
const STRUCTURE: &str = "https://ck.uesp.net/wiki/Script_File_Structure";
const FLAGS: &str = "https://ck.uesp.net/wiki/Flag_Reference";
const ARRAYS: &str = "https://ck.uesp.net/wiki/Array_Reference";
const OPERATORS: &str = "https://ck.uesp.net/wiki/Operator_Reference";

fn entry(
    title: &'static str,
    description: &'static str,
    example: Option<&'static str>,
    reference_url: &'static str,
) -> Entry {
    Entry {
        title,
        description,
        example,
        reference_url,
    }
}

/// Select by dialect and syntactic use, rather than importing another game's keyword set.
pub(super) fn skyrim(word: &str, context: SyntaxKind) -> Option<Entry> {
    Some(match word {
        "int" => entry(
            "Int",
            "A signed 32-bit integer, from -2,147,483,648 to 2,147,483,647. Its default value is 0. Integer literals use decimal digits or the hexadecimal prefix 0x.",
            Some("Int count = 12"),
            LITERALS,
        ),
        "float" => entry(
            "Float",
            "A 32-bit, single-precision floating-point number. Its default value is 0.0. Decimal fractions may be rounded to the nearest representable value.",
            Some("Float delay = 1.25"),
            LITERALS,
        ),
        "bool" => entry(
            "Bool",
            "A Boolean value: True or False. Its default value is False. Conditions can also use values convertible to Bool.",
            Some("Bool enabled = True"),
            LITERALS,
        ),
        "string" => entry(
            "String",
            "Text enclosed in double quotes, with an empty string as the default value. Escape sequences represent newlines, tabs, quotes, and backslashes. Skyrim compares and caches strings without regard to case; runtime casing can differ from the spelling in source.",
            Some("String message = \"Ready\\nGo\""),
            LITERALS,
        ),
        "true" => entry(
            "True",
            "The Boolean literal representing a true condition.",
            None,
            LITERALS,
        ),
        "false" => entry(
            "False",
            "The Boolean literal representing a false condition.",
            None,
            LITERALS,
        ),
        "none" => entry(
            "None",
            "The absence of an object or array reference. Object and array variables default to None. Compare a reference with None to check whether it is assigned.",
            Some("If target != None\n    target.Activate(Self)\nEndIf"),
            LITERALS,
        ),
        "scriptname" => entry(
            "ScriptName",
            "Begins the script header and declares its name. The script name must match its PSC filename. Optional Extends and flags follow the name.",
            Some("ScriptName Example Extends ObjectReference"),
            STRUCTURE,
        ),
        "extends" => entry(
            "Extends",
            "Declares the parent script. The child inherits its members and can override functions or events with matching signatures.",
            Some("ScriptName Example Extends ObjectReference"),
            STRUCTURE,
        ),
        "import" => entry(
            "Import",
            "Makes another script's global functions callable without the script-name prefix. It does not create an instance or import instance members.",
            Some("Import Utility\n\nFunction Pause()\n    Wait(1.0)\nEndFunction"),
            STRUCTURE,
        ),
        "function" => entry(
            "Function",
            "Declares a callable unit with parameters and an optional return type. Omitting the return type means the function returns no value. Non-native functions have a body ending with EndFunction.",
            Some("Int Function Double(Int value)\n    Return value * 2\nEndFunction"),
            FUNCTIONS,
        ),
        "endfunction" => entry(
            "EndFunction",
            "Closes the body of a non-native function.",
            None,
            FUNCTIONS,
        ),
        "event" => entry(
            "Event",
            "Declares an event handler with no return value. Receiving an engine event requires the expected name and parameter types; declaring an arbitrary event does not make the game send it.",
            Some("Event OnInit()\nEndEvent"),
            EVENTS,
        ),
        "endevent" => entry(
            "EndEvent",
            "Closes the body of a non-native event handler.",
            None,
            EVENTS,
        ),
        "global" => entry(
            "Global",
            "Marks a function callable through its script type without an instance. Global functions have no Self or Parent and cannot directly access instance members.",
            Some("Int Function Double(Int value) Global\n    Return value * 2\nEndFunction"),
            FUNCTIONS,
        ),
        "native" => entry(
            "Native",
            "Declares a function or event implemented by the runtime, with no Papyrus body or closing EndFunction/EndEvent. The declaration alone does not supply that runtime implementation.",
            Some("Function Wait(Float seconds) Global Native"),
            FUNCTIONS,
        ),
        "property" => entry(
            "Property",
            "Declares a member that other scripts can read or write through accessors. A full property defines Get and/or Set functions; Auto and AutoReadOnly provide generated accessors.",
            Some("Int Property Count Auto"),
            PROPERTIES,
        ),
        "endproperty" => entry(
            "EndProperty",
            "Closes a full property containing explicit Get and/or Set accessors. Auto properties do not use this terminator.",
            None,
            PROPERTIES,
        ),
        "auto" if context == SyntaxKind::StateDecl => entry(
            "Auto",
            "Selects the script's initial state. A script can have only one auto state. Entering that initial state does not send OnBeginState.",
            Some("Auto State Ready\nEndState"),
            STATES,
        ),
        "auto" => entry(
            "Auto",
            "On a property, generates a backing variable and both Get and Set accessors. Before State, selects the script's initial state.",
            Some("Int Property Count = 0 Auto"),
            PROPERTIES,
        ),
        "autoreadonly" => entry(
            "AutoReadOnly",
            "Generates a property that can be read but cannot be assigned in-game. It requires a constant initializer.",
            Some("Int Property Limit = 8 AutoReadOnly"),
            PROPERTIES,
        ),
        "hidden" => entry(
            "Hidden",
            "A standard Skyrim declaration flag. On a script, hides it from the normal Creation Kit attachment list; on a property, hides it from the property window. It does not make the property private.",
            Some("Int Property Count Auto Hidden"),
            FLAGS,
        ),
        "conditional" => entry(
            "Conditional",
            "A standard Skyrim declaration flag that exposes script variables to the Creation Kit condition system. The script must be Conditional too. On an Auto property, it marks the generated backing variable.",
            Some("ScriptName Example Conditional\nInt Property Count Auto Conditional"),
            FLAGS,
        ),
        "state" => entry(
            "State",
            "Declares a named runtime state whose functions and events can override the empty-state implementations. GotoState changes the active state; GetState returns its name.",
            Some("Auto State Ready\nEndState"),
            STATES,
        ),
        "endstate" => entry(
            "EndState",
            "Closes a named state and its function/event implementations.",
            None,
            STATES,
        ),
        "self" => entry(
            "Self",
            "The current script instance in a non-global function or event. Its type is the script that owns the body.",
            Some("target.Activate(Self)"),
            FUNCTIONS,
        ),
        "parent" => entry(
            "Parent",
            "Calls a parent-script implementation on the current instance, bypassing the child override. Available only in non-global functions or events of a script with a parent.",
            Some("Parent.OnInit()"),
            FUNCTIONS,
        ),
        "return" => entry(
            "Return",
            "Stops the current function or event immediately. A function with a return type supplies a compatible value; a function or event without one uses bare Return.",
            Some("Return value * 2"),
            STATEMENTS,
        ),
        "if" => entry(
            "If",
            "Runs a branch when its condition converts to True. Optional ElseIf branches are checked in order, and Else handles the remaining case. EndIf closes the statement.",
            Some("If count > 0\n    count -= 1\nElse\n    count = 0\nEndIf"),
            STATEMENTS,
        ),
        "elseif" => entry(
            "ElseIf",
            "Tests another condition when all preceding If/ElseIf branches were false. Only the first matching branch runs.",
            None,
            STATEMENTS,
        ),
        "else" => entry(
            "Else",
            "Runs the fallback branch when no preceding If/ElseIf condition was true.",
            None,
            STATEMENTS,
        ),
        "endif" => entry(
            "EndIf",
            "Closes an If statement, including any ElseIf and Else branches.",
            None,
            STATEMENTS,
        ),
        "while" => entry(
            "While",
            "Repeats its body while the condition converts to True. The condition is evaluated before each iteration, so the body may never run. EndWhile closes the loop.",
            Some("While count > 0\n    count -= 1\nEndWhile"),
            STATEMENTS,
        ),
        "endwhile" => entry(
            "EndWhile",
            "Closes a While loop and returns control to its condition.",
            None,
            STATEMENTS,
        ),
        "new" => entry(
            "New",
            "Creates a one-dimensional array with elements initialized to their type's default value. Skyrim's New syntax requires a constant integer size from 1 through 128. It does not construct script objects.",
            Some("Int[] values = New Int[8]"),
            ARRAYS,
        ),
        "length" => entry(
            "Length",
            "The read-only Int property giving the number of elements in an array. Skyrim returns 0 for a None array. Valid indices run from 0 through Length - 1.",
            Some("Int count = values.Length"),
            ARRAYS,
        ),
        "as" => entry(
            "As",
            "Explicitly converts an expression to the type on its right. A valid downcast of an object reference returns None when the instance does not have the requested child type.",
            Some("Float amount = count As Float"),
            "https://ck.uesp.net/wiki/Cast_Reference",
        ),
        _ => return None,
    })
}

/// Operators have token-local help even when surrounding code is incomplete.
pub(super) fn operator(kind: SyntaxKind, unary: bool) -> Option<Entry> {
    use SyntaxKind::*;
    let (title, description, example) = match kind {
        Equals => (
            "=",
            "Assigns the value on the right to the destination on the left. In a declaration it introduces an initializer or parameter default; in a call it labels a named argument. Equality uses ==.",
            Some("count = 4"),
        ),
        Plus if unary => (
            "+",
            "Unary plus preserves the numeric operand's value.",
            None,
        ),
        Minus if unary => (
            "-",
            "Unary minus negates a numeric operand.",
            Some("Int offset = -4"),
        ),
        Plus => (
            "+",
            "Adds numbers or concatenates strings. Folio also accepts an Int or Float alongside a String by converting the number to text.",
            Some("String label = \"Count: \" + count"),
        ),
        Minus => (
            "-",
            "Subtracts the right numeric operand from the left.",
            Some("Int remaining = total - used"),
        ),
        Star => ("*", "Multiplies numeric operands.", None),
        Slash => (
            "/",
            "Divides numeric operands. Int division discards the fractional part. Division by zero produces a runtime error.",
            Some("Int half = count / 2"),
        ),
        Percent => (
            "%",
            "Returns the remainder of integer division. Float operands are not supported. A zero divisor produces a runtime error.",
            Some("Int remainder = count % 3"),
        ),
        PlusEq => (
            "+=",
            "Adds to the destination, or appends to a String, then assigns the result back. A property update uses both its Get and Set accessors.",
            Some("count += 1"),
        ),
        MinusEq => (
            "-=",
            "Subtracts from the destination and assigns the result back. A property update uses both its Get and Set accessors.",
            Some("count -= 1"),
        ),
        StarEq => (
            "*=",
            "Multiplies the destination and assigns the result back. A property update uses both its Get and Set accessors.",
            None,
        ),
        SlashEq => (
            "/=",
            "Divides the destination and assigns the result back. Int division discards the fractional part; a property update uses both its accessors.",
            None,
        ),
        PercentEq => (
            "%=",
            "Assigns the integer division remainder back to the destination. Float operands are not supported.",
            None,
        ),
        EqEq => (
            "==",
            "Tests whether two values are equal and returns Bool. Skyrim String comparisons ignore case. Assignment uses =.",
            Some("Bool empty = count == 0"),
        ),
        NotEq => (
            "!=",
            "Tests whether two values are unequal and returns Bool. Skyrim String comparisons ignore case.",
            Some("Bool assigned = target != None"),
        ),
        Less => (
            "<",
            "Returns True when the left value is less than the right value.",
            None,
        ),
        Greater => (
            ">",
            "Returns True when the left value is greater than the right value.",
            None,
        ),
        LessEq => (
            "<=",
            "Returns True when the left value is less than or equal to the right value.",
            None,
        ),
        GreaterEq => (
            ">=",
            "Returns True when the left value is greater than or equal to the right value.",
            None,
        ),
        Bang => (
            "!",
            "Converts its operand to Bool and negates it.",
            Some("Bool disabled = !enabled"),
        ),
        AndAnd => (
            "&&",
            "Returns True only when both operands convert to True. Short-circuits: the right operand is evaluated only when the left is true.",
            Some("If target != None && target.IsEnabled()\nEndIf"),
        ),
        OrOr => (
            "||",
            "Returns True when either operand converts to True. Short-circuits: the right operand is evaluated only when the left is false.",
            None,
        ),
        LParen | RParen => (
            "()",
            "Groups an expression to control evaluation order, or encloses the parameters/arguments of a function declaration or call.",
            Some("Int result = (2 + 3) * 4"),
        ),
        LBracket | RBracket => (
            "[]",
            "Declares an array type, specifies a New array's size, or accesses an element by its zero-based index. Skyrim arrays are one-dimensional.",
            Some("Int[] values = New Int[8]\nInt first = values[0]"),
        ),
        Comma => (
            ",",
            "Separates parameters in a declaration or arguments in a call.",
            None,
        ),
        Dot => (
            ".",
            "Accesses a property or calls a function on a reference. Script-name qualification is also used for global functions.",
            Some("Utility.Wait(1.0)"),
        ),
        _ => return None,
    };
    Some(entry(title, description, example, OPERATORS))
}
