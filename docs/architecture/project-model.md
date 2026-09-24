# 项目与依赖

`folio.toml` 是 Folio 的项目边界。命令从当前目录向上查找最近的同名清单，或使用 `--manifest-path` 指定；文件名须精确匹配。一个工作区有一个源码根和一个输出目录。相对路径以声明它的清单所在目录为基准。

当前只接受 schema 3、Skyrim Papyrus、`skyrim-se` 目标和 PEX 输出。下面的示例同时展示默认目录和可选的项目设置：

```toml
schema = 3

[package]
name = "example-mod"
version = "0.1.0"

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

`paths.source` 和 `paths.output` 默认分别为 `Source/Scripts` 和 `Scripts`。两者须是工作区内互不重叠的相对目录，不得指向 `.folio` 或经符号链接逃出工作区。源码目录必须存在，输出目录由构建创建。清单拒绝未知字段。`profile` 参与构建身份，目前不切换优化级别。

`user-flags` 显式声明自定义 Papyrus flags；`Hidden` 与 `Conditional` 是内置项。`fill-missing-arguments` 默认关闭，省略没有默认值的必填参数会报错；开启时在调用点补类型默认值并发出警告，不修改函数签名。`debug-info` 默认开启 PEX 函数与行映射。两项设置均进入构建指纹。lint 规则严重性支持 `off`、`info`、`warning` 和 `error`，不改变 `check/build` 的编译合法性。

## 本地依赖与声明

`[[dependencies]]` 按清单顺序声明。`kind = "package"` 指向本地包目录或清单；`kind = "psc"` 指向无需清单的 PSC 源码目录；`kind = "pex"` 指向编译后的 PEX 目录；`kind = "sdk"` 指向声明 JSON；`kind = "builtin"` 使用内置声明 ID：

```toml
[[dependencies]]
name = "ck-1.6.1170"
kind = "builtin"
path = "ck-1.6.1170"

[[dependencies]]
name = "skse-2.2.8"
kind = "builtin"
path = "skse-2.2.8"

[[dependencies]]
name = "other-mod"
kind = "psc"
path = "../OtherMod/Source/Scripts"
```

本地源码包、PSC 目录与 SDK 只参加名称和类型分析；`build` 仅为最终选中的根工作区脚本生成 PEX。PSC 目录递归读取 UTF-8 `.psc`，在内存中提取 API 声明，不写 JSON、不编译依赖脚本；空目录、重名脚本或无效声明会报错。其依赖身份由 `name` 指定，版本在元数据中标记为 `local`，不具有传递依赖。API 可见不意味着 Folio 部署相应运行时。Folio 不从网络获取依赖、不进行版本求解，也不建立用户级包 registry。

PEX 目录依赖尚属实验性，只在根项目显式开启后可用：

```toml
[experimental]
pex-dependencies = true

[[dependencies]]
name = "compiled-mod"
kind = "pex"
path = "../CompiledMod/Scripts"
```

Folio 递归读取目录内的 Skyrim PEX 3.1/3.2，为脚本、继承、变量、属性、状态和可调用成员提取 API；不执行或部署这些 PEX。PEX 不保存参数默认值，也不标明可调用成员原本是事件还是函数。传齐参数的调用可用；省略参数会报出无法确定默认值的诊断，即使开启 `fill-missing-arguments` 也不会猜测；源码覆盖继承自 PEX 的此类成员会报错。损坏或当前 codec 不支持的 PEX 明确拒绝。PEX 来源没有 PSC 定义位置。`folio tree` 和 `folio metadata` 会显示它与其他来源共同参与的整脚本选择。

同名脚本按整脚本选择：后列依赖优先，包自身源码优先于其依赖，根项目源码最高。传递依赖按声明顺序展开；同一来源多次出现时，最后一次决定优先级。目录扫描顺序不参与选择。包内部的大小写不敏感重名和依赖环会报错。`folio tree` 与 `folio metadata` 展示提供者、选中来源和外部运行要求。

路径身份与 Papyrus 脚本名身份分别处理。脚本名按 ASCII 大小写不敏感规则比较，源码文件名与 `ScriptName` 由共享语法结果核对。符号引用只在明确解析的依赖闭包内查找。

## 声明载体

Folio 读取 schema 1 和 2 的 UTF-8 JSON 声明；`folio declarations generate` 从明确指定的 PSC 目录生成 schema 2，也可输出可复现 gzip。声明保留脚本继承、flags、imports、状态、变量、函数、事件、属性、参数默认值和相对来源位置。载体中的类型名由语义层解析，不另建一套类型兼容规则。

`folio declarations list` 列出内置包。内置 CK 1.6.1170 与 SKSE 2.2.8 包需要在项目中显式选择；上例中 SKSE 的同名脚本覆盖 CK。SKSE 声明可见仍要求运行环境具有匹配实现。生成器与内置资源的来源信息见[声明来源记录](../../modules/formats/declarations/builtin/README.md)。

项目输入在一次操作中形成一致快照。构建指纹包含源码、依赖与声明内容、脚本选择和影响语义的配置；发布前会复核输入。产物缓存与发布行为见[构建与产物](build-artifacts.md)。
