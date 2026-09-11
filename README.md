# osu2ugc

一个用 Rust 编写的命令行工具，把 **osu!mania 4K** 的 `.osu` 谱面转换成 UMIGURI Chart v8 的 `.ugc` 文本谱面。生成的文件也可以导入 Margrete 继续编辑。

## 轨道映射

每条 osu!mania 轨道固定占用 UMIGURI 的 4 个单位宽度，四轨正好铺满整个 16 宽地面：

| osu!mania 轨道 | UGC 横坐标 | 宽度 | 覆盖范围 |
| --- | ---: | ---: | --- |
| 1（最左） | 0 | 4 | 0–3 |
| 2 | 4 | 4 | 4–7 |
| 3 | 8 | 4 | 8–11 |
| 4（最右） | C（十进制 12） | 4 | 12–15 |

普通音符转换为 TAP，长按音符转换为 HOLD。BPM 变化、拍号和 osu!mania 的 SV 变速点也会转换。

## 编译

安装 Rust 1.74 或更高版本后，在项目目录运行：

```powershell
cargo build --release
```

Windows 可执行文件位于 `target\release\osu2ugc.exe`。

## 使用

```powershell
# 自动在输入文件旁生成同名 .ugc
osu2ugc.exe "歌曲 [难度].osu"

# 指定输出路径
osu2ugc.exe "歌曲 [难度].osu" "输出.ugc"

# 在每个 LN 终点额外添加 DAMAGE 音符
osu2ugc.exe --damage-ln-end "歌曲 [难度].osu" "输出_damage.ugc"
```

工具只接受 `Mode: 3` 且 `CircleSize: 4` 的谱面。`.osu` 文件需为 UTF-8 编码，支持 UTF-8 BOM。

## 转换说明

- `.ugc` 输出遵循 UMIGURI Chart v8 格式，UTF-8 编码、LF 换行。
- osu! 背景图会写入 `BGIMG`；音频文件名和首个红线偏移会写入 `BGM`、`BGMOFS`。
- 使用 `--damage-ln-end` 时，每个 HOLD 终点会额外生成一个同位置、同宽度的 DAMAGE 音符；默认关闭。
- `Version` 中形如 `[12]` 的数字会用作等级；无法识别时等级默认为 `1`。
- UGC 的拍号只能在小节线上切换。若输入谱面在小节中途改变拍号，工具会给出警告并保持前一个拍号。
- 建议转换后用 Margrete 导入检查一次复杂变速谱面。
