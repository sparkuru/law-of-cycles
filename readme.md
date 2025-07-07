# law-of-cycles
円環の理（Law of Cycles），園心繫所有的魔女，焰心繫園。

说白了，就是要写一个 mihomo-cli，满足自己使用习惯的

## shellscript

1. 使用 `#!/bin/sh` 作爲 shebang，這是大多數 shell 的最小兼容子集
2. 使用 posix 標準語法
   1. 使用 `[`、`test`
   2. 定義和使用變量時，`KEY="value"`、`${KEY}` 或 `"$KEY"`
   3. 使用 `/bin/true` 代替 `true`

## refer

1. 本家，[MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo.git)
2. docs，[虛空終端 Doc](https://wiki.metacubex.one/)
3. [Dreamacro/clash](https://github.com/Dreamacro/clash)
4. gui，[mihomo-party-org/mihomo-party](https://github.com/mihomo-party-org/mihomo-party.git)
