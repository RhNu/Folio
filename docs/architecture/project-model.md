# 项目与依赖

`folio.toml` 是 Folio 的项目边界。命令从当前目录向上查找最近的同名清单，或使用 `--manifest-path` 指定；文件名须精确匹配。一个项目有一个源码根和一个输出目录。文件系统相对路径以清单所在目录为基准。

清单使用当前版本的字段与默认值，不声明 schema 版本，并拒绝未知字段。当前支持 Skyrim Papyrus、`skyrim-se` 目标和 PEX 输出：

```toml
[package]
name = "example-mod"
version = "0.2.0"

[paths]
source = "Source/Scripts"
output = "Scripts"

[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]
user-flags = ["ProjectFlag"]
fill-missing-arguments = false

[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
debug-info = true

[lint.rules]
"papyrus.prefer-truthy-none-check" = "warning"
```

`paths.source` 和 `paths.output` 默认分别为 `Source/Scripts` 和 `Scripts`。两者须是项目内互不重叠的相对目录，不得指向 `.folio`，也不得包含符号链接或其他重解析点。源码目录必须存在，输出目录由构建创建。`profile` 参与构建身份，目前不切换优化级别。

`user-flags` 显式声明自定义 Papyrus flags；`Hidden` 与 `Conditional` 是语言内置项。`fill-missing-arguments` 默认关闭，省略没有默认值的必填参数会报错；开启时在调用点补类型默认值并发出警告，不修改函数签名。`build.debug-info` 默认开启 PEX 函数与行映射。两项设置均进入构建指纹。lint 规则严重性支持 `off`、`info`、`warning` 和 `error`，不改变 `check/build` 的编译合法性。

## 依赖输入与选择

`[[dependencies]]` 按清单顺序声明，每项都是一个独立的 API 来源。支持四种模式：

| kind | path | 装载行为 |
| --- | --- | --- |
| `psc` | PSC 目录 | 递归提取脚本 API；默认 UTF-8，可指定 `encoding = "windows1252"` |
| `pex` | PEX 目录 | 实验性地还原二进制中存在的 API 事实 |
| `decl` | 声明文件 | 按文件内容读取 JSON 或 `.fdecl` 二进制声明 |
| `repo` | 仓库内的逻辑路径 | 从全局本地仓库定位声明文件，再使用相同的声明解码与分析流程 |

`name` 是清单内唯一的来源别名，不要求与声明内容匹配。依赖不声明包版本，也不加载其他项目的清单或传递依赖。`psc`、`pex`、`decl` 的路径可以指向项目外、另一磁盘上的本地输入；主机路径与可移植的来源身份分别保存。`encoding` 只适用于 `psc`。

```toml
[[dependencies]]
name = "ck"
kind = "repo"
path = "ck/1.6.1170.0"

[[dependencies]]
name = "skse"
kind = "repo"
path = "skse/2.2.8"

[[dependencies]]
name = "other-mod"
kind = "psc"
path = "../OtherMod/Source/Scripts"

[[dependencies]]
name = "shared-api"
kind = "decl"
path = "../Declarations/shared-api.fdecl"
```

四种输入都投影成同一种声明模型，再进入统一的脚本选择和语义分析。依赖的函数体不进入编译输入，也不产生本次可部署代码；根项目保留源码与函数体，`build` 只为根项目脚本生成 PEX。PSC 与 PEX 目录不能为空，目录内大小写不敏感的重名脚本或无效声明会报错。PSC 直接依赖与声明生成共用目录扫描、文本解码和 API 提取流程。目录载体的内部文件和子目录须是普通文件系统条目，拒绝链接或特殊类型。

同名脚本按整脚本选择：后列依赖优先，根项目源码最高。即使不同别名指向同一载体，也保留每次依赖声明的顺序和来源；目录枚举顺序不参与优先级。脚本名按 ASCII 大小写不敏感规则比较，PSC 文件名须与 `ScriptName` 一致。`folio tree` 和 `folio metadata` 展示提供者、选中来源和外部运行要求。API 可见性不代表运行时实现已安装；Folio 不下载或部署依赖，不执行版本求解。

## 全局本地仓库

`FOLIO_HOME` 默认是用户目录下的 `.folio`，仓库位于其 `repo` 子目录。可设置绝对路径的环境变量改变位置；应用在启动时固定本次操作使用的位置：

```powershell
$env:FOLIO_HOME = 'D:\Folio'
```

`repo` 的逻辑路径使用 `/` 分隔，不接受绝对路径、盘符、反斜线、空组件、`.` 或 `..`，也不允许通过链接逃出 `FOLIO_HOME/repo`。例如 `ck/1.6.1170.0` 查找：

```text
$FOLIO_HOME/repo/ck/1.6.1170.0.fdecl
$FOLIO_HOME/repo/ck/1.6.1170.0.json
```

只有一个候选文件时使用它；两个都存在时报告歧义，要求在清单中显式指定 `.fdecl` 或 `.json` 后缀。明确指定后缀时只读取该文件。候选文件的出现、消失及链接目标变化都属于输入变化。`folio declarations list` 递归列出本地仓库中可解码的声明文件及来源、profile 和脚本数。

## 实验性的 PEX 输入

直接读取 PEX 目录需由根项目显式开启：

```toml
[experimental]
pex-dependencies = true

[[dependencies]]
name = "compiled-mod"
kind = "pex"
path = "../CompiledMod/Scripts"
```

Folio 读取 Skyrim PEX 3.1/3.2，为脚本、继承、变量、属性、状态和可调用成员提取 API。PEX 不保存参数默认值，也不标明可调用成员原本是事件还是函数。这些信息以逐参数的 `unknown` 默认值和 `unknown-callable` 成员明确保存。传齐参数的调用可用；省略未知默认值的参数会报错，即使开启 `fill-missing-arguments` 也不会猜测；源码覆盖继承的未知种类可调用成员会报错。未知事实在 JSON、二进制声明及语义输入之间保持一致。声明文件不因来源标签而要求开启实验选项。

损坏或当前 codec 不支持的 PEX 明确拒绝。PEX 及预生成声明不具有消费机器上的 PSC 定义位置。PSC 目录依赖可使用本次装载的真实源码快照进行符号定位。

## 声明模型与载体

声明协议从 schema 1 开始，仅接受当前格式。JSON 顶层包含格式标识、协议版本、语言/ABI profile、生成来源和脚本；来源不承担包身份或运行时能力判断：

```json
{
  "format": "folio-declarations",
  "schema": 1,
  "profile": "papyrus-skyrim",
  "origin": { "source": "self-authored-api" },
  "scripts": [
    {
      "name": "Example",
      "members": [
        {
          "name": "Run",
          "kind": "function",
          "return_type": "Int",
          "native": true,
          "parameters": [
            { "name": "count", "ty": "Int", "default": { "kind": "literal", "value": "1" } }
          ]
        }
      ]
    }
  ]
}
```

`origin.source` 是可移植的描述标签；生成器还写入 `input_digest`，摘要涵盖排序后的相对输入路径与解码后的源码文本。脚本可保存继承、native、flags、imports、状态、成员及相对来源位置。来源位置只是历史生成信息，不被当作消费机器上的可跳转路径。类型名由语义层解析。

成员按 `function`、`event`、`unknown-callable`、`property`、`variable` 分别存储有效事实；函数省略 `return_type` 表示无返回值。属性的 `access` 为 `auto`、`auto-read-only` 或含 `readable`、`writable` 的 `manual`。参数默认值为 `required`、带 Papyrus 字面量文本的 `literal` 或 `unknown`；省略 `default` 等同于 `required`。参数顺序有语义意义。

JSON 与 `.fdecl` 使用相同模型和校验器，按内容识别编码。二进制格式是固定数组与数字标签的 MessagePack，再进行 raw DEFLATE 压缩。60 字节头部依次包含 8 字节 `FOLDECL\0`、小端 u32 schema、小端 u64 原始长度、小端 u64 压缩长度和 32 字节原始 payload BLAKE3；其后是完整压缩流。解码核对长度、摘要、完整流消费、结构及未知标签，拒绝尾随数据。载体与解压数据各最多 128 MiB，单字符串最多 1 MiB，单容器最多 100,000 项，结构深度最多 64，值数量也有限制。协议数组顺序和标签定义在 `formats/declarations` 中，不使用 Rust 内存布局作为文件协议。

## 从源生成声明

生成命令显式指定来源、编码和目的地，默认输出二进制。`--repo` 指定仓库逻辑路径；`--output` 指定任意目标文件路径，两者恰好选一个。生成拒绝覆盖已有文件，并在复核源码快照后发布完整文件：

```powershell
folio declarations generate --source-root '<CK PSC 目录>' --source ck/1.6.1170.0 --encoding windows1252 --repo ck/1.6.1170.0
folio declarations generate --source-root '<SKSE PSC 目录>' --source skse/2.2.8 --repo skse/2.2.8
folio declarations generate --source-root '<自有 PSC 目录>' --source my-api --format json --output my-api.json
```

CK 和 SKSE 是分别生成的 API 输入，需要在项目中显式选择。取得相应工具及脚本来源的用户可以在自己的本地仓库生成声明；源码及派生声明的使用与分发仍遵循各自来源的条件。生成来源应保留实际工具版本与输入摘要，版本名称由用户选择的仓库路径表达。

项目输入在一次操作中形成一致快照。构建语义指纹与文件快照分别处理，详细契约见[构建与产物](build-artifacts.md)。
