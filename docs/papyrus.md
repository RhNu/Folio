# Folio 的 Skyrim Papyrus 规范

本文描述 Folio 当前识别和检查的 Skyrim Papyrus 行为。项目须选择 `languages.papyrus.dialect = "skyrim"` 与 `build.target = "skyrim-se"`。语法接受范围、目标可生成范围和实际运行时 API 是不同条件；`check` 会报告无法分析或无法在目标上表达的程序。

## 源码与声明

源码使用 `.psc`。脚本以 `ScriptName` 声明，可指定 `Extends`、`Hidden`、`Conditional`。Folio 识别导入、变量、属性、具名状态、函数、事件、参数默认值、`Global`、`Native` 和项目声明的自定义 flags。脚本名与文件名按 ASCII 大小写不敏感规则核对；同包重名报错。

函数与事件体支持局部变量、`Return`、赋值、`If`/`ElseIf`/`Else`、`While`、调用和表达式。表达式包括字面量、算术与比较、逻辑操作、成员访问、数组构造与索引，以及显式 `As` 转换。赋值 `=` 与相等比较 `==` 分别处理。词法树保留空白、分号行注释、`;/ … /;` 块注释、`{ … }` 文档注释、续行及错误节点。损坏的局部结构不会丢弃整份文件；不能安全分析或生成的节点会产生诊断。

## 类型、调用与转换

分析层统一处理内建类型、脚本引用和数组。它解析继承、成员、函数调用、参数绑定和来源；未解析名称使用错误状态限制连锁诊断，不伪装成正常 `None`。外部 SDK 声明只提供可见 API，不能替代游戏或扩展的实际运行时。

Skyrim 的 `If`、`ElseIf`、`While` 及 `&&`、`||`、`!` 接受 `Bool`，也允许 `Int`、`Float`、`String`、脚本引用、数组和 `None` 的隐式 Bool 转换。转换在语义结果中显式记录，生成时形成 MIR `Cast`。这不会放宽 `None` 比较：数值、`Bool` 和 `String` 不能因此与 `None` 比较。数组可用于条件和 `None` 比较，但两种写法的行为不据此视为等价。

`String` 与 `Int`/`Float` 可用 `+` 拼接，`String +=` 也接受对应数值。语义层确定转换，降级时对数值执行 String `Cast`，再使用 `StrCat`。不能表达的操作会明确拒绝；例如不能把浮点取模静默改为另一种计算。

调用按已解析的形参绑定命名实参，同时保留源码中的求值顺序。没有默认值的必填实参缺失时默认报 `semantic.argument-count`。项目开启 `fill-missing-arguments` 后，在调用点补入类型默认值并报告 `semantic.argument-defaulted`；已有实参仍照原顺序求值。

## Skyrim 内置操作

`GetState()` 和 `GotoState(String)` 是编译器内置实例方法，Folio 为状态读取与迁移生成代码；迁移依次执行 `OnEndState()`、更新状态、`OnBeginState()`。数组的 `Length`、`Find` 和 `RFind` 使用目标数组操作。引擎 native 方法须经项目源码或 SDK 声明可见；内置状态和数组能力不使其他引擎脚本自动可见。

语义错误下，编辑器仍可取得已解析的局部类型、定义和诊断。`build` 只接受通过语义、目标与 MIR 验证的程序。具体阶段和求值顺序保证见[编译管线](architecture/compiler.md)；项目设置与依赖可见性见[项目与依赖](architecture/project-model.md)。
