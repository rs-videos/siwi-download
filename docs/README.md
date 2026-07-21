# Siwi Download 文章集

面向开发者与社区推广的文章合集。可投稿到掘金、知乎、rustcc、V2EX、SegmentFault、微信公众号等中文技术社区。

## 文章列表

| # | 文章 | 受众 | 适合平台 |
|---|---|---|---|
| 1 | [《siwi-download：一个用 Rust 写的轻量下载器》](./01-introduction.md) | 所有开发者 | 掘金 / 知乎 / V2EX / 公众号 |
| 2 | [《5 分钟玩转 siwi-download：CLI + Rust 库上手指南》](./02-getting-started.md) | 想立刻用起来的人 | 掘金 / 知乎 / dev.to |
| 3 | [《拆解 siwi-download：Rust 异步下载器是怎么炼成的》](./03-deep-dive.md) | Rust 开发者 | rustcc / 知乎 / 掘金 |

## 使用建议

- **首发平台**：掘金、知乎、rustcc（中文 Rust 社区聚集地）
- **标题本地化**：发布时可根据平台风格微调标题
- **配图**：建议自行补充终端截图（`-P` 进度条、`-j` JSON 输出）会极大提升转化率
- **转载**：署名 + 保留仓库链接 `https://github.com/rs-videos/siwi-download`

## 推广渠道参考

| 港口 | 定位 | 备注 |
|---|---|---|
| 掘金 | 综合技术社区 | 流量大，配封面图 |
| 知乎「Rust 语言」话题 | Rust 开发者 | 长尾流量好 |
| rustcc.cn | 中文 Rust 社区 | 专业读者 |
| V2EX `/go/rust` | 程序员社区 | 讨论氛围浓 |
| 微信公众号 | 私域流量 | 需排版 |
| HelloGitHub | 开源推荐 | 月度精选投稿 |

## 文章配图素材建议

1. **封面图**：siwi-download Logo + Rust 齿轮 + 下载箭头（可用 Excalidraw / Figma 制作）
2. **终端截图**：
   - `siwi-download https://... -P` 带进度条下载
   - `siwi-download https://... -j` JSON 输出
3. **架构图**（用于第 3 篇）：
   - HEAD → Range → Chunk 写入流程
   - tokio runtime 异步时序

## 推广 checklist

发布前检查：
- [ ] 文中所有代码示例本地跑通
- [ ] 版本号与最新 release 一致（当前 v2.0.0）
- [ ] 仓库链接、star 按钮引导
- [ ] 配图齐全
- [ ] 社区规范遵守（掘金要求原创标记、知乎禁止外链过多等）
