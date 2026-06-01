# Cronymax 扩展信任 / 安全模型(v1 alpha)

一句话:**安装即信任作者**。装一个 cronymax 扩展 = 在你机器上运行该作者的代码,权限与你自己跑 `node script.js` 一样。

配套:[`permission-removal.md`](permission-removal.md)(为什么撤回 permission model 的决策记录)· [`developer-guide.md`](developer-guide.md) · [`spec-v0.3.md`](spec-v0.3.md) §6/§7。

---

## 1. 没有能力沙箱

v1 alpha **撤回了** Node 26 Permission Model(决策见 `permission-removal.md`)。因此:

- 扩展进程的 Node API **完整**:`fs` / `child_process` / `net` / `http` / worker threads / 原生 `.node` 模块 **全开**。
- 平台**不 emit** `--permission` / `--allow-*` 任何标志。
- 安装弹窗只问「由 \<publisher\> 提供,是否安装」——**没有** per-capability 清单、没有风险分级。
- manifest 里的 `capabilities.fs` / `capabilities.network` / `process` 等字段是**惰性**的(被接受但不强制)。

**唯一**有实际作用的 capability 是事件总线白名单:`capabilities.events.subscribe` / `events.emit` 决定扩展能订阅 / 发布哪些 `cronymax.*` 平台 topic(这是平台 RPC 路由层的门控,不是 OS 沙箱)。

信任边界 = **install-time**。用户在安装那一刻信任扩展作者。和 VS Code 扩展、npm 包、shell 脚本同一类信任。

> 给用户的实践建议:只装你信任来源的扩展。`.cmx` 包 = 纯 ZIP,装前可解开看 manifest 与 `dist/`。

---

## 2. 确实存在的隔离

撤回 capability 沙箱不等于零隔离。以下是真实的、平台强制的边界:

### 2.1 per-extension 进程隔离(崩溃隔离)
每个 host-backed 扩展跑在**自己的 Node 子进程**里。一个扩展崩溃 / 卡死 / OOM **不波及**其他扩展或 cronymax 主程序。这是 cronymax 相对单进程扩展宿主的差异化点。

平台监督每个 host(P10-T01):

- 崩溃 / 卡死 → **自动重启**,带退避(立即 → 500ms → 2s)。
- 连续超过 **3 次** → **停用**该扩展 + toast 通知;设置里显示 **Disabled (crashed)**,可手动 Enable 重试。
- 内存超阈值(默认 ~1.5 GB)→ **只告警**(日志 + 一次 toast),**不杀**(P10-T02 缩为观测,与信任模型一致——不做硬资源强杀)。

### 2.2 webview origin 隔离
扩展的 webview(操作视图 / content renderer)走自定义 scheme `cronymax-webview://<extId>/...`,**每个扩展是自己的浏览器 origin**(host = extId):

- 浏览器原生 same-origin 策略**自动**拒绝跨扩展 `postMessage` / DOM 访问,无需平台手工 gate。
- 默认 CSP:`default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src cronymax-webview: data:; connect-src 'self'; font-src 'self' data:`。扩展默认**连不了外网**;要外网通信走 Node 侧 `main.js` 再 `postMessage` 进 iframe,或在 manifest `content.renderer.csp.connect_src` 显式放开。
- scheme handler 用 `realpath` 做前缀校验,挡 `../` / symlink 越狱。

注意:webview 的 iframe origin 隔离与扩展 Node 侧的 `capabilities.network`(已惰性)是**两套、两个 origin**——webview 默认锁外网,Node 侧完全放开。

### 2.3 `cronymax.*` 命名空间保留
publisher 为 `cronymax` 的扩展 id / topic 被平台保留(RPC 路由层),扩展不能冒充平台来源。这是路由防混淆,不是 OS 强制。

### 2.4 存储 / secrets 按扩展隔离
每扩展私有 `storagePath` / `globalStoragePath`;`secrets` 按 namespace 隔离,跨扩展不可读。

---

## 3. 不被沙箱的东西(明确)

- 文件系统(扩展可读写你能访问的任何路径)。
- 网络(Node 侧 `fetch`/`net` 任意出站)。
- 子进程 / 原生模块。
- CPU / 进程数(无硬额度;只有内存**观测告警**,不强杀)。

这些都遵循「装它 = 信任作者」。要更强的隔离(sandbox-exec / seccomp / AppContainer)是 v1 之后的可选 γ 阶段工作(spec §7),不在 alpha。

---

## 4. 日志可见性(排错,不是安全审计)

`host.log` / `output.log` / `channels/*.log` / `extension-host.log` 记录扩展行为供**排错**。Node 26 Permission Model + `audit.log` 已随 permission 撤回——这些日志是运维排错用途,**不是** security audit。`cronymax diagnostic-bundle` 收集时会脱敏(`$HOME → ~`、`Bearer <token> → REDACTED`)。

---

## 小结

| 项 | v1 alpha |
|---|---|
| Node API | 完整,不沙箱 |
| `--permission` / `--allow-*` | 不 emit |
| 安装弹窗 | 只问「是否安装(by publisher)」 |
| 信任边界 | install-time 作者信任 |
| 有作用的 capability | 仅 `events.subscribe` / `events.emit` 白名单 |
| 进程隔离 | ✅ per-extension host(崩溃隔离 + 重启≤3→停用) |
| 内存 | 观测告警,不强杀 |
| webview | ✅ per-extension origin + CSP 锁外网 |
| secrets / storage | ✅ 按扩展隔离 |
