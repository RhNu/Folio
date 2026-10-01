# CLI、格式化、lint 与编辑器

CLI 与 LSP 使用同一项目解析和分析视图。工具层不复制 Papyrus 的名称解析或类型系统。进程入口负责日志、输出格式与协议；库返回结构化结果。

## 仓库维护命令

`modules/apps/xtask` 是开发维护入口，不属于面向 Papyrus 项目的 `folio` 命令。仓库根目录的 Cargo alias 允许运行 `cargo xtask lines`。该命令扫描 `modules` 中的 `.rs` 文件，按含 Rust 内容的行计数；空行和纯注释行不计，代码后的注释不使该行被排除，字符串字面量中的内容计入代码。单文件超过 650 行为 warning，超过 1200 行为 error 并返回状态 2；warning 不影响退出状态。结果按路径排序并给出汇总。超过硬限制时按职责拆成子模块；若内嵌测试使文件触及阈值，可把测试移到独立测试子模块。

## CLI 与机器输出

`folio` 接受全局 `--manifest-path`、`--log-filter` 和 `--log-format`。项目命令包括 `init`、`new`、`metadata`、`tree`、`check`、`build`、`fmt`、`lint`、`inspect`、`lsp` 和 `declarations`。实际参数以 `folio --help` 及子命令帮助为准。

`metadata` 默认输出 JSON；`tree`、`check`、`build`、`lint` 和 `inspect` 默认文本，可选择 JSON。项目 metadata、依赖树及构建结果分别有版本化 schema。标准输出承载所选数据，日志写标准错误；`folio lsp` 的标准输出只用于 LSP 报文。成功返回状态 0，项目或源码错误返回 2，I/O 与进程错误返回 1。

## 格式化

`folio fmt` 与 `folio fmt --check` 检查根项目的 Papyrus 源码；存在差异时列出文件并返回状态 2。只有 `folio fmt --write` 写回。格式化使用无损 CST，不要求依赖 API 齐备或语义构建成功。四空格缩进、基本 token 空格及末尾换行是当前样式；保留已有空行、各行原有换行类型、注释与字符串内容，不改变关键字和标识符拼写。

候选结果重新解析并核对非空白 token。语法损坏、token 改变或无法证明安全时拒绝该文件的修改。写回前复核输入内容；LSP 文档格式化使用同一格式化器，并在缓冲区版本或项目 generation 改变时拒绝过期结果。

## lint

`folio lint` 和 LSP 使用同一 typed HIR 规则。规则 `papyrus.prefer-truthy-none-check` 默认以 warning 提示：在 `If`、`ElseIf`、`While` 条件中，已解析为脚本引用的 `value != None` 可以直接写成 `value`，反向比较也适用。数组、未解析类型和其他类型不触发此建议。当前只报告诊断，不自动修改源码。

清单中的 `[lint.rules]` 可把该规则设为 `off`、`info`、`warning` 或 `error`。warning 不使命令失败；设为 error 时返回状态 2。禁用 lint 不会关闭 `check` 或 `build` 的合法性诊断。

## 语言服务器与 VS Code

`folio lsp` 经 stdio 提供项目诊断、hover、定义与声明跳转、签名提示、文档符号、语义 token 和全文格式化。hover 显示符号名、类型或函数签名、所属脚本及所选脚本提供者的包与来源。签名提示使用解析后的调用与形参顺序；文档符号列出脚本、状态及成员。打开的未保存 `.psc` 缓冲区优先于磁盘；关闭后恢复磁盘状态。服务器处理文档变化、保存和受监控文件事件，过期结果不作为最新诊断发布。位置默认使用 UTF-16，客户端支持时可协商 UTF-8。只有位于根项目源码目录内的新建未保存脚本会加入项目图；根项目源码与 PSC 目录依赖可在真实源码中定位；依赖符号定位只解析本次装载的 PSC 快照，不将依赖函数体加入语义分析。`decl`、`repo` 和 PEX 只展示可核实的别名或声明载体来源，不返回历史生成路径构造的文件跳转。

服务器在会话内保存磁盘项目输入与不可变分析视图。打开、重开或提交相同文本时复用语义结果，只更新该文档诊断的版本；实际缓冲区编辑重新投影缓存的项目输入，保留其他打开的覆盖层。关闭时重读该文件以恢复磁盘内容，新建未保存文件和已删除文件的关闭重新解析项目图。保存及文件监控事件刷新清单、源码与声明依赖；加载失败清除当前结果并保留输入监控，修复输入后可继续使用同一会话。仓库监控覆盖无后缀引用的两种候选文件；不存在的路径通过最近的已存在父目录监控其创建。支持动态相对路径监控的客户端会收到清单、源码目录和依赖载体的监控注册，包含工作文件夹外的依赖；其他客户端需自行转发这些文件变化。分析缓存只驻留内存，不在 `.folio` 保存 LSP 语义数据。源码或语义设置变化仍会重新计算全项目语义。

文档大纲、语义 token 和诊断共用按文本构建的行索引转换协议位置，保留 UTF-8／UTF-16 和 CRLF 边界。`debug` 日志记录消息处理及项目加载、投影、语义分析、诊断发布和符号查询的 `elapsed_us`；项目 span 不携带整份 metadata。`modules/apps/lsp/tests/performance.py` 可用 release 二进制生成独立的首次打开、关闭、重开及编辑耗时报告，生成输入和日志位于可丢弃的 `.folio/lsp-perf`，`--compare` 核对另一份报告中的完整符号响应摘要。

`editors/vscode` 中的客户端注册 `.psc`、提供 Papyrus 默认文件图标和基础 TextMate 语法高亮、启动 `folio lsp` 并转发项目文件事件。图标随 VSIX 打包；当前文件图标主题未为 `.psc` 或 Papyrus 指定专用图标且允许语言图标时使用。可执行文件按扩展设置、`PATH`、开发模式下的仓库 `target/debug` 顺序查找。语义 token 使用 VS Code 的通用 token 类型和修饰符，按主题规则显示已解析的函数、事件、类型、属性、参数和变量。一个客户端会话对应一个 Folio 项目文件夹。扩展可用 `npm run package` 生成供本地安装的 VSIX；`npm run publish` 是 Marketplace 发布入口，VSIX 不包含 Folio 可执行文件。源码、调试与安装步骤见[扩展说明](../../editors/vscode/README.md)。
