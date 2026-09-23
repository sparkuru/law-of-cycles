# law-of-cycles

円環の理（Law of Cycles），園心繫所有的魔女，焰心繫園。

实现一个 mihomo-cli，满足自己使用习惯


## refer

1. 本家，[MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo.git)
2. docs，[虛空終端 Doc](https://wiki.metacubex.one/)
3. [Dreamacro/clash](https://github.com/Dreamacro/clash)
4. gui，[mihomo-party-org/mihomo-party](https://github.com/mihomo-party-org/mihomo-party.git)

## 使用

命令名为 `kami`，使用 Rust + Ratatui / Crossterm，Tokio 负责异步通信。

```console
# 本机 Rust 1.90+，或用 ./hako cargo build --release --locked
cargo build --release --locked
./target/release/kami --help
./target/release/kami tui --demo
./target/release/kami --json status
```

构建后的 `kami` 是原生可执行程序，无需 Python。无参数运行显示帮助；
传入包含控制器地址的配置文件或使用 `--controller` 后自动打开 TUI，也可显式运行 `kami tui`。

连接已有的 Mihomo 控制器。启动 TUI 前需通过 `--controller`、环境变量
`KAMI_CONTROLLER` 或配置文件指定地址；控制器需要密钥时，再设置 `KAMI_SECRET`
或从配置文件读取。未指定 controller 时不会打开 TUI。单次 CLI 命令仍默认使用
`http://127.0.0.1:9090`。例如直接读取目标正在使用的配置：

```console
./target/release/kami --mihomo-config /path/to/mihomo.yaml
./target/release/kami --mihomo-config /path/to/mihomo.yaml status
./target/release/kami --controller http://127.0.0.1:9090
```

`--config xxx.yaml` / `xxx.yml` 也支持相同读取方式。YAML 只用于提取 API
地址和密钥，不会被修改，也不会启动或重载 Mihomo。

支持四页 TUI、节点切换与测速、连接和日志、运行时模式/TUN 切换、
本机 systemd 管理，以及 `env` / `exec` 代理环境。

TUI 支持侧栏导航、列表/详情分栏、鼠标点选、右键操作菜单和拖动调整分栏。
`tui --demo` 可在没有 Mihomo 内核时体验界面，不改动实际网络和服务。

安装、配置和快捷键见 [使用说明](docs/usage.md)，
产品范围与架构见 [设计文档](docs/design.md)。
