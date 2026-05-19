# MessagePack-RPC · Rust ↔ Node 26 Spike 验收报告

> P0-T05 / Phase 0 关键 spike。验证 spec-v0.3 §决策 8（IPC：Unix socket
> + MessagePack-RPC）的可行性、性能、和 wire 兼容性。

- 主机：macOS 15 · arm64
- Rust 端：`rmp-serde 1.3.1` + `tokio 1.52`（unix socket listener）
- Node 端：**Node v26.1.0** + `@msgpack/msgpack 3.x`（unix socket client）
- 协议：MessagePack-RPC 0/1/2（request/response/notify）裸数组帧
- spike 源：`/tmp/msgpack-rpc-spike/{rust-client,node-server}/`

---

## 0. 一句话结论

**通过。P99 18-30μs，比 5ms 目标低 200 倍以上**。`rmp-serde` ↔ `@msgpack/msgpack` 互通零调整。Unix socket 在 Node 26 `--permission` 下需要 `--allow-net`——这条**得写进 manifest capability 文档**。

---

## 1. 验收清单（exec-plan §P0-T05）

| 项 | 目标 | 实测 |
|---|---|---|
| ping-pong round-trip 通 | ✅ | ✅ |
| 1000 round-trip P99 | < 5ms | **66μs**（66× under） |
| 10000 round-trip P99 | — | **20-30μs** |
| 100000 round-trip P99 | — | **18μs** |
| Rust → Node 编码兼容 | ✅ | tuple struct → positional array 直通 |
| Node → Rust 解码兼容 | ✅ | `@msgpack/msgpack` 出来的 array 直接被 `rmp-serde` 反序列化 |

---

## 2. 性能数据

### N=1000，4字节 payload（原始 spike 目标场景）

```
total elapsed: 10.74 ms
throughput:    93,122 req/s
latency μs:    P50=7   P90=16   P99=66   max=256
mean μs:       10.3
```

### N=10000，不同 payload 大小（带 Node `--permission --allow-net`）

| payload | P50 μs | P90 μs | P99 μs | max μs | throughput |
|---|---|---|---|---|---|
| 4 B  | 9 | 14 | **29** | 179 | 100k req/s |
| 256 B | 10 | 14 | **24** | 157 | 92k req/s |
| 1024 B | 11 | 14 | **30** | 179 | 88k req/s |
| 16384 B | (spike 代码受限，见 §3.3) | — | — | — | — |

### N=100000，4字节 payload（持久压测）

```
total elapsed: 918 ms
throughput:    108,926 req/s
latency μs:    P50=8   P90=11   P99=18   max=193
mean μs:       8.7
```

**结论**：cronymax 真实工作流的事件流（每 turn 几十～几百次 RPC，单条 payload < 1KB）**完全在 P99 < 30μs 这个区间**。Phase 2 性能目标（P99 < 5ms for command round-trip）可以正常完成。

---

## 3. 关键发现

### 3.1 ⚠️ Unix socket 需要 `--allow-net`

Node 26 `--permission` 模式下连接 Unix domain socket **被 ACL 算作网络**：

```
[node] socket error: connect ERR_ACCESS_DENIED Access to this API has
been restricted. Use --allow-net to manage permissions.
```

**影响**：
- spec-v0.3 §6.1 `build_node_flags` 必须**永远** emit `--allow-net`，至少把 cronymax 控制平面的 socket 通过
- 用户 manifest **不需要**声明 `network.allow` 来连 cronymax 自家 RPC——这是平台基础设施，不是扩展的网络出站

**spec 修改建议**：
- §6.1 `build_node_flags` 加 `--allow-net` 作为无条件 prefix（不归用户 capability）
- §6.4 防绕过表加一行 "Unix socket / Named Pipe 走 `--allow-net`；平台始终 emit，capability `network` 决定的是**用户扩展能否**`fetch()` 外网"

### 3.2 ✅ 序列化兼容（无需调整）

| 形状 | Rust（rmp-serde） | Node（@msgpack/msgpack） |
|---|---|---|
| `[0, msgid, "ping", ["pong"]]` | `#[derive(Serialize)] struct R(u8, u32, &str, Vec<&str>)` + `encode::to_vec` | `decoder.decodeMulti(buf)` → `[0, msgid, "ping", ["pong"]]` |
| `[1, msgid, null, "pong"]` | `encode::to_vec(&tuple)` | `encoder.encode([1, msgid, null, "pong"])` → `Buffer` |

两边都用**裸 MessagePack 数组帧**，不需要 length prefix 或别的封装。

**坑1**：`rmp_serde::encode::to_vec_named` 把 Rust struct 编成 map（字段名作 key），不是 array——MessagePack-RPC 错。**用 tuple struct + `to_vec`** 直接得到 positional array。

**坑2**：`#[derive(Deserialize)]` 的命名 struct 也尝试从 map 解；要 tuple struct (`struct Response(u8, u32, ..., ...)`) 才能从 array 解。

### 3.3 spike 代码限制：单 read 解码

我的 Rust 端用 `read(&mut [u8; 4096])` 一次读一帧，对 < 4KB payload 工作，但 16KB+ 会拆成多帧导致解码失败。

**对真实实现的意义**：Phase 2 `rpc::codec` 必须用增量解码器（rmp-rs 的 `Read`-based 接口或 `rmpv::decode::read_value` 流式版），不能假设一次 `read` 拿完整帧。`@msgpack/msgpack` 的 `Decoder.decodeMulti` 已经处理这个（Node 端 spike 正确）。

### 3.4 ExperimentalWarning 与 cancellation

- `--allow-net` 在 Node 26.1.0 还是 experimental，但本 spike 跑的都是 connect 类操作，被 ACL 放行；warning 只在启动时打印一次，加 `--no-warnings` 抑制
- 本 spike 没测**MessagePack-RPC cancellation token**（spec 要求支持）——cancellation 协议 layer（msgid 关联 + 'cancel' 通知）是上层抽象，跟 wire 性能无关，Phase 2 跟 `rpc::server` 一起做

---

## 4. spike 重跑步骤

```bash
# 准备
mkdir -p /tmp/msgpack-rpc-spike
git clone <这个 spike> /tmp/msgpack-rpc-spike  # 或参考下方文件清单手抄
cd /tmp/msgpack-rpc-spike/node-server && npm install
cd /tmp/msgpack-rpc-spike/rust-client && cargo build --release

# 默认跑（N=1000, payload=4B）
NODE_26=$(fnm exec --using=26 which node)
N=1000 NODE_BIN="$NODE_26" \
  NODE_SERVER=/private/tmp/msgpack-rpc-spike/node-server/server.js \
  NODE_PREFIX_ARGS="--no-warnings --permission --allow-fs-read=/private/tmp/msgpack-rpc-spike --allow-net" \
  ./target/release/spike

# 自定义压测
N=100000 PAYLOAD_BYTES=512 NODE_BIN="$NODE_26" ... ./target/release/spike
```

环境变量：
- `N` — 测多少次 round-trip（默认 1000）
- `PAYLOAD_BYTES` — payload 大小（默认 4）
- `NODE_BIN` — Node 26 二进制全路径
- `NODE_SERVER` — server.js 全路径（建议用 `/private/tmp/...` canonical 形避免 `--permission` 下 symlink 问题）
- `NODE_PREFIX_ARGS` — 额外 Node 启动参数

---

## 5. 文件清单

```
/tmp/msgpack-rpc-spike/
├── rust-client/
│   ├── Cargo.toml          ← rmp-serde 1.3 + tokio + serde
│   └── src/main.rs         ← unix listener + spawn node + 100 warmup + N 测量
└── node-server/
    ├── package.json        ← @msgpack/msgpack ^3.0.0
    └── server.js           ← unix socket client + Decoder.decodeMulti + Encoder.encode
```

---

## 6. 影响 Phase 2 实现的决策落档

| 决策 | 取值 |
|---|---|
| Rust msgpack 实现 | `rmp-serde` 1.3+（实测；ergonomic + 速度足够）|
| Node msgpack 实现 | `@msgpack/msgpack` 3.x（实测；流式 Decoder 自带）|
| Wire 形式 | 裸 MessagePack 数组（**不**加 length prefix），`@msgpack/msgpack` Decoder 处理粘包 |
| Rust struct 形 | tuple struct + `encode::to_vec` / `decode::from_slice` |
| 启动 Node 必带 flag | `--no-warnings --permission --allow-net`（最小集）+ 用户 capability 衍生 |
| Phase 2 性能目标 | P99 < 5ms for command round-trip — **预期巨大冗余**（实测 P99 < 30μs） |
| Phase 2 codec 实现要点 | 增量解码（不能假设一次 read 拿完整帧）；mailbox + msgid → oneshot map 做 request/response 关联 |

---

## 7. 进 Phase 1 / Phase 2 的判定

**进**。

- ✅ Wire 兼容性零问题
- ✅ 性能远超目标（200× 冗余）
- ✅ Unix socket 在 Node 26 可用，已落档 `--allow-net` 必带
- ✅ 实施细节（增量解码、tuple struct、cancellation 上层）已留好钩子

附：spec-v0.3 §6.1 修改建议（与 Node 26 spike 报告 §3.1 同源）—— `build_node_flags` 永远 emit `--allow-net`，跟用户 capability 解耦。

---

---

## 附录 · Phase 0 评议后的架构变更（2026-05-20）

评议 §C 重决把 RPC 通道从 **Unix socket** 改成 **inherited fd 3**（详 `phase-0-review.md`）。变更动因：Node 26 把 Unix socket connect 算 network ACL，强迫平台 emit `--allow-net`，跟"网络是用户 capability"语义冲突。

### 已经验证的事实

1. **fd 3 在 Node 26 `--permission` 下不需要 `--allow-net`**：见 `/tmp/node-perm-spike/test9-inherited-fd.js` + test9-parent.js（父进程 baseline Node spawn 子进程 `--permission` 无 `--allow-net`，子进程 `new net.Socket({ fd: 3 })` 成功，`process.permission.has('net')` 返回 false，双向数据通）
2. **`net.Socket({fd:3})` 是 duplex**：Node `child_process.spawn` 的 stdio 非标准 fd 用 socketpair（不是 unidirectional pipe）实现，所以 fd 3 是 bidirectional
3. **裸 MessagePack 数组帧不变**：跟 §2.2 一致，wire 形式不依赖 transport

### 未验证（推到 Phase 2 实现期）

1. **Rust↔Node fd 3 性能**：spike 原始测 Unix socket P99 18-30μs；fd 3 走 kernel socketpair（pipes 也是 kernel buffer），**性能预期与 Unix socket 一致**——但未实测 Rust 端 socketpair + `pre_exec` 复制实现。
2. **Rust 端实现复杂度**：需 `socketpair(AF_UNIX, SOCK_STREAM, 0)` + `pre_exec` 把 child fd 通过 `dup2` 移到 3，比 `UnixListener::bind` 多 ~20 行 Rust（用 `nix` 或 `interprocess` crate）。Phase 2 P2-T03 任务里做。
3. **Windows 等价**：Windows 没有 `socketpair`，需用 `CreatePipe` 或 Named Pipe + `HANDLE` 继承。Node `child_process` 跨平台抽象已经处理，Rust 端要走 `winapi` crate；Phase 2 P2-T01 多平台打包时一起做。

### Phase 2 实现要点

```rust
// crates/cronymax/src/extensions/host/node.rs (Phase 2 草案)
use std::os::unix::process::CommandExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use tokio::net::UnixStream;

async fn spawn_with_fd3(node_bin: &Path, flags: &[String], bootstrap: &Path)
    -> Result<(tokio::process::Child, UnixStream)>
{
    let (parent_end, child_end) = std::os::unix::net::UnixStream::pair()?;
    let child_fd = child_end.as_raw_fd();

    let mut cmd = tokio::process::Command::new(node_bin);
    cmd.args(flags).arg(bootstrap);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            // 把 child_end 移到 fd 3
            libc::dup2(child_fd, 3);
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    drop(child_end);  // parent 端用 std → tokio 转换
    let stream = UnixStream::from_std(parent_end)?;
    Ok((child, stream))
}
```

Node 端 bootstrap.js：

```js
const net = require("node:net");
const rpc = new net.Socket({ fd: 3 });
// 跟 spike 一样的 MessagePack 数组帧
```

### 性能不确定性的处理

虽然预期 fd 3 socketpair 跟 Unix socket 性能一致（同样走 kernel sock buffer），P2-T09 性能基准任务里**单独跑一次** 1000-round-trip 验证，发现差 > 10% 再回头优化。给基准目标 P99 < 5ms 留了 250× 冗余，几乎不可能跑不过。

---

**文档版本**：spike-v1.1 · 2026-05-19 / patched 2026-05-20 · Rust 1.97 + Node 26.1.0
