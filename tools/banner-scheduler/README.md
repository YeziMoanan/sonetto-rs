# Sonetto 卡池排期工具

这是一个完全本地的静态页面。页面读取 `summon_pool.json`、安排卡池时间并导出 JSON，不会打开或修改 SQLite 数据库。数据库操作由固定的 `Apply-BannerPreset.ps1` 完成，而且默认只做 Dry Run。

“全活动卡池”只筛选 `bannerFlag = 2` 的召唤池，不会修改 `assets/static/activity/activity_infos.json`，也不会开启游戏活动总表。

## 打开页面

直接打开 `tools/banner-scheduler/index.html`，或在 PowerShell 中运行：

```powershell
Start-Process tools\banner-scheduler\index.html
```

页面不依赖网络，也不需要安装 Node 包。

## 生成预设

1. 选择或拖入 UTF-8 格式的 `sonetto-data/excel2json/summon_pool.json`。
2. 使用名称、ID 前缀、`bannerFlag`、`type` 和选择状态进行筛选。
3. 使用“全部卡池”“全联动”“全复刻”“全活动卡池”或“全限定”预设，或逐池选择。
4. 在已选队列中拖动、上下移动或按源顺序、ID、Priority 排序。
5. 选择一种排期模式：
   - 同时开放：全部池从同一天零点开放到 SQLite 32 位时间上限。
   - 按批次轮换：按队列顺序，每批若干池、持续若干天，上一批关闭时下一批开放。
   - 逐池手动日期：分别设置每个池的上线和下线日期。
6. 导出 JSON。该文件可再次导入页面编辑。

浏览器自定义预设保存在当前页面来源的 `localStorage` 中，只保存 ID、顺序、筛选器和排期输入，不复制 210 条源数据。换浏览器或清理站点数据后需要重新导入 JSON。

## 数据库 Dry Run

先在 `sonetto-rs` 目录运行不带 `-Apply` 的命令：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\banner-scheduler\Apply-BannerPreset.ps1 `
  -PresetPath C:\path\to\banner-schedule.json `
  -DatabasePath runtime\db\sonetto.db `
  -SummonPoolJson D:\python-tools\重返未来1999\sonetto-data\excel2json\summon_pool.json
```

Dry Run 会校验：

- JSON 版本、上海时区、连续顺序、唯一池 ID 和时间范围；
- 预设源池数量与当前 `summon_pool.json` 一致；
- 每个预设 ID 都存在于当前源文件；
- 数据库包含 `banner_schedule` 和 `user_summon_pools`。

它只输出即将应用的时间表，不创建备份，也不写数据库。

## 明确应用

确认 Dry Run 输出后，额外加入 `-Apply`：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\banner-scheduler\Apply-BannerPreset.ps1 `
  -PresetPath C:\path\to\banner-schedule.json `
  -DatabasePath runtime\db\sonetto.db `
  -SummonPoolJson D:\python-tools\重返未来1999\sonetto-data\excel2json\summon_pool.json `
  -Apply
```

执行器会在数据库旁创建 `sonetto.db.banner-preset.<UTC时间>.bak`，再用一个事务精确替换 `banner_schedule`，同步已存在的同 ID `user_summon_pools`，最后逐行校验 ID 和时间。重复应用同一个预设会得到相同结果。

`-SkipBackup` 仅用于隔离测试或操作者明确决定跳过备份的场景；正常运行不要使用。

## 服务重启顺序

`gameserver` 启动时会从 `common/Config.toml` upsert 其中列出的卡池，并可能覆盖这些池在数据库中的时间。执行器不会修改该配置文件。

可重复的运行顺序是：

1. 启动或重启 `sdkserver` 和 `gameserver`。
2. 确认 `gameserver` 已监听 `127.0.0.1:23301`。
3. 先执行 Dry Run，再使用 `-Apply` 重放 JSON 预设。
4. 重连或重新载入客户端。

以后每次重启 `gameserver`，都应在服务就绪后重新应用预设。

## 自动测试

```powershell
node --test tools/banner-scheduler/tests/*.test.js
powershell -NoProfile -ExecutionPolicy Bypass -File tools\banner-scheduler\tests\Test-ApplyBannerPreset.ps1
```

PowerShell 测试只创建系统临时目录和临时 SQLite 数据库，不会操作 `runtime/db/sonetto.db`。
