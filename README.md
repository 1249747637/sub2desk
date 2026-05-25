# Sub2Desk

Sub2Desk 是一个面向 sub2api 管理员的 Windows 桌面工具，用来快速把上游 OpenAI/Anthropic 兼容账号添加到指定分组。项目使用 Tauri 2、Vue 3 和 TypeScript 构建。

## 功能

- 顶部 Profile 横条，默认 3 个 Profile，支持新增、删除、切换和改名。
- 每个 Profile 独立保存平台、池模式、池重试次数、优先级、分组、Base URL、上游 API Key、账号名称和模型选择。
- 账号名称可自定义；留空时发送时自动使用时间生成名称。
- 支持拉取 OpenAI/Anthropic 兼容 `/v1/models` 模型列表，默认全选。
- 每个模型支持单独测试，测试内容为“你好”。
- 每个模型支持禁用/启用，禁用后保留在列表但不会写入 `model_mapping`。
- 提交前有摘要确认，提交时调用 sub2api Admin API 创建 `apikey` 上游账号。
- 设置页支持后端管理地址、管理员 API Key、日间/夜间模式。
- 管理员 API Key 保存到系统安全凭据；其他状态保存到本地配置文件。

## 直接运行

已构建的 Windows 可执行文件在：

```text
release/Sub2Desk.exe
```

双击即可运行。关闭窗口时程序会直接退出，不保留后台进程。

## 社区

感谢 [LINUX DO 社区](https://linux.do/) 的讨论和使用反馈。这个工具主要面向 sub2api 管理员的日常账号维护场景，也欢迎在社区里交流使用问题、部署坑和改进建议。

## 使用前配置

打开右上角设置，填写：

- 后端管理地址：只填 origin，例如 `https://api.example.com`
- 管理员 API Key：sub2api 后台的 Admin API Key
- 主题：日间模式或夜间模式

后端管理地址不要填写：

- `/api/v1`
- `/admin`
- 管理后台页面完整路径
- 前端页面路由

程序内部会自动拼接 `/api/v1`。

## 创建账号流程

1. 选择或新建一个 Profile。
2. 填写账号名称。可留空，留空时会自动生成时间名。
3. 选择平台：OpenAI 或 Anthropic。
4. 填写上游 Base URL 和上游 API Key。
5. 刷新并选择分组。
6. 设置优先级，默认 `1`。
7. 按需打开池模式，默认池重试次数 `3`。
8. 点击“获取模型列表”。
9. 测试或禁用模型。
10. 点击“发送”，确认后创建 sub2api 上游账号。

## 常见坑

### 后端地址尽量填 HTTPS

如果填写 `http://...`，服务器通常会 301 跳转到 `https://...`。旧版 HTTP 客户端可能会在跟随 301 时把 `POST` 变成 `GET`，导致创建接口返回账号列表 `data.items`，而不是创建结果。

当前版本已经对创建请求做了保留 POST 的重定向处理，但仍建议直接填写：

```text
https://你的域名
```

### 创建成功但看不到账号

优先检查：

- 设置里的后端地址是否是 sub2api 后端 origin。
- 管理员 API Key 是否是 Admin API Key，不是用户 API Key。
- <img width="1855" height="855" alt="image" src="https://github.com/user-attachments/assets/3cab6eea-7b8d-462a-b4bb-505d0f3b1e22" />

- 选择的分组平台是否和 Profile 平台一致。
- sub2api 后台账号列表是否有筛选条件。
- 反代是否把 `POST /api/v1/admin/accounts` 改写成了 `GET` 或转到了前端页面。

### 模型测试气泡为空

部分 OpenAI 兼容模型会把内容放在 `reasoning_content`，而 `message.content` 为空。Sub2Desk 已兼容这种响应，会优先显示 `content`，为空时显示 `reasoning_content`。

### 不同电脑需要重新填 Admin API Key

管理员 API Key 使用系统凭据保存，不写入项目文件。换电脑或换系统用户后需要重新填写。

### 自签证书或过期证书

如果后端使用自签或过期 HTTPS 证书，请先修复证书。Sub2Desk 默认使用系统 TLS 校验，不跳过证书验证。

## sub2api 提交字段

Sub2Desk 创建账号时调用：

```http
POST /api/v1/admin/accounts
x-api-key: <admin-api-key>
Idempotency-Key: <uuid>
```

请求体核心结构：

```json
{
  "name": "account-name",
  "platform": "openai",
  "type": "apikey",
  "credentials": {
    "api_key": "sk-xxx",
    "base_url": "https://api.example.com",
    "pool_mode": false,
    "pool_mode_retry_count": 3,
    "model_mapping": {
      "model-a": "model-a"
    }
  },
  "extra": {},
  "group_ids": [1],
  "concurrency": 1,
  "priority": 1,
  "confirm_mixed_channel_risk": true
}
```

## 开发

需要 Node.js、npm、Rust 和 Tauri 2 构建环境。

安装依赖：

```bash
npm install
```

开发预览：

```bash
npm run dev
```

前端构建：

```bash
npm run build
```

构建 release exe：

```bash
cd src-tauri
cargo build --release
```

生成的二进制在：

```text
src-tauri/target/release/sub2desk.exe
```

如果要更新仓库里的便携版：

```bash
copy src-tauri\target\release\sub2desk.exe release\Sub2Desk.exe
```

## 数据位置

- Profile 和普通设置：系统配置目录下的 `Sub2Desk/state.json`
- 管理员 API Key：系统凭据管理器

## 安全说明

- 不要把真实管理员 API Key、上游 API Key 或服务器密码提交到仓库。
- `release/Sub2Desk.exe` 是便携二进制，但本地运行后仍会在系统配置目录和凭据管理器写入用户自己的配置。
- 管理员 API Key 权限很高，只建议在可信电脑上使用。
