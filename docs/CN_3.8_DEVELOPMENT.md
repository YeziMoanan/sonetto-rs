# 国服 3.8 开发基线

## 固定客户端

- APK：`D:\python-tools\重返未来1999\重返未来：1999.apk`
- 包名：`com.shenlan.m.reverse1999`
- 版本：`3.8.0` / `190`
- SHA-256：`EA6CD8AD7FAAFE6EDA42F4C2073DCF1BDA7F24AAE9A7011FDFE918AFFF69D3C0`

## 本地服务

- SDK HTTP：`127.0.0.1:21100`
- 游戏 TCP：`127.0.0.1:23401`
- 数据库：`runtime/db/sonetto-3.8-cn.db`

所有服务只监听 loopback。不得修改 Windows、WinHTTP、环境变量或 Android 代理设置。

## 验证基线

```powershell
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo build -p sdkserver -p gameserver
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\tests\Test-PrepareCn38Runtime.ps1
```

## 准备运行目录

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\Prepare-Cn38Runtime.ps1 `
    -DataSource "D:\python-tools\重返未来1999\sonetto-data" `
    -RepositoryRoot $PWD.Path `
    -RuntimeRoot (Join-Path $PWD.Path "runtime") `
    -Profile debug
```

脚本从 `sonetto-data/excel2json` 复制配置表，从本仓库的 `assets/static` 复制静态响应。初始 `runtime/data` 是国际服 3.6 数据的独立副本，只用于 3.8 服务启动和协议分析，不能作为大厅适配完成时的最终 3.8 数据。

运行目录首次创建时，`runtime/db` 必须为空。启动服务后产生的数据库只属于 3.8，不得复制回国际服 3.6 目录。
