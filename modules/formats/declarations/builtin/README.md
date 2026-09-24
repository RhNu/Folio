# 内置声明来源

本目录的 gzip 文件是 Folio schema 2 声明快照，仅保存从指定 PSC 提取的 API 事实，不包含完整函数体或原始源码。CK 与 SKSE 分别生成。

| 包 | PSC 数 | 源内容 BLAKE3 | 压缩文件 SHA-256 |
| --- | ---: | --- | --- |
| `ck-1.6.1170` | 14,301 | `c7591f7ac0874c21a07fa59499cd98d5cbf240011cdf4cb12c6ceac194fb1055` | `14ffb5df984540370004270f038dec2ee70422dfbbf68e60ebc0b25460480988` |
| `skse-2.2.8` | 62 | `f7a877dbc99e74fc16e5672ef07388eca7072ae3193502761328fcce4d86a675` | `c05566edbcc45a43e679c4ae38dcd3944753e883e453ab8126a1db05d312fc56` |

持有相应来源的用户可以向新文件重新生成：

```powershell
folio declarations generate --source-root <CK PSC 目录> --name ck-1.6.1170 --version 1.6.1170 --source ck-1170-source --encoding windows1252 --output ck-regenerated.json.gz --gzip
folio declarations generate --source-root <SKSE PSC 目录> --name skse-2.2.8 --version 2.2.8 --source skse-1170-source --output skse-regenerated.json.gz --gzip
```

生成命令拒绝覆盖已有文件。源文件的分发条款尚需核实；对外分发这些派生资源前须确认权利人、使用条件和归属要求。
