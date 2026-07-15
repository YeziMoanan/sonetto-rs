# 默认全角色池 `summon.json`

本目录中的 `summon.json` 是国际服 3.6 的完整召唤候选文件。它保留原文件全部 1050 条记录，只重建官方常驻角色池 `ID 2（Amid the Water）` 的五条稀有度记录。

## 内容

池 2 包含 `character.json` 中全部 `isOnline = "1"` 的 120 个角色：

| 文件 rare | 游戏显示 | 角色数 |
| ---: | --- | ---: |
| 5 | 6 星 | 61 |
| 4 | 5 星 | 29 |
| 3 | 4 星 | 17 |
| 2 | 3 星 | 11 |
| 1 | 2 星 | 2 |

其中包含常驻、限定、联动和特殊可用角色。未上线的 Schneider `3029` 和 Machine D III `9998` 不在文件中。

## 概率与保底

此文件只改变各稀有度的候选名单，不修改服务端概率或保底：

- 6 星基础概率 1.5%；
- 5 星 8.5%；
- 4 星 40%；
- 3 星 45%；
- 2 星 5%；
- 保留现有 6 星递增概率和 70 抽硬保底；
- 十连仍保证至少一个 5 星或以上角色；
- 池 2 没有 UP，同稀有度角色等概率。

客户端卡池详情页可能继续显示官方静态说明，但实际抽取候选由服务端加载的本文件决定。

## 重新生成

在 `sonetto-rs` 目录运行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\banner-scheduler\New-DefaultAllCharactersSummon.ps1 `
  -SummonJson ..\sonetto-data\excel2json\summon.json `
  -CharacterJson ..\sonetto-data\excel2json\character.json `
  -OutputPath tools\banner-scheduler\presets\default-all-characters\summon.json `
  -Force
```

生成器会严格读取 UTF-8，拒绝把输出写到任一输入路径，使用同目录临时文件验证后再替换目标，并报告总数及各稀有度数量。

验证命令：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\banner-scheduler\tests\Test-DefaultAllCharactersSummon.ps1
```

## 部署边界

生成和测试不会部署本文件。`gameserver` 从其可执行文件旁 `config.toml` 的 `[paths].excel_data` 目录加载名为 `summon.json` 的文件，并且只在进程启动时读取。

当前 `target/debug/config.toml` 指向：

```text
D:\python-tools\重返未来1999\sonetto-data\excel2json
```

后续如需应用，必须单独执行以下流程：

1. 停止 `gameserver`。
2. 确认实际使用的 `config.toml` 和 `excel_data` 路径。
3. 备份活动目录中的原 `summon.json`。
4. 将本文件复制到该目录并保持文件名为 `summon.json`。
5. 启动 `gameserver`，进入官方常驻池 2 做受控测试。

本任务没有执行这些部署步骤，也没有修改 `sonetto-data`、运行数据库或任何服务进程。

## 回滚

停止 `gameserver`，恢复第 3 步的原 `summon.json`，再重新启动服务。只恢复文件但不重启不会重新加载配置。
