# E2E 测试手动执行步骤

## 前置条件

1. PostgreSQL 数据库运行中
2. 已安装 agent-browser: `cargo install agent-browser`

## 测试步骤

### 终端 1: 启动 Admin Server (后端 API)

```bash
cd crates/admin-server
DATABASE_URL="postgresql://palpo:password@localhost/palpo" cargo run --release
```

等待看到：
```
Admin Server listening on http://0.0.0.0:8081
```

### 终端 2: 启动 Admin UI (前端)

```bash
cd crates/admin-ui
dx serve
```

等待看到：
```
Serving at: http://127.0.0.1:8080/
```

### 终端 3: 重置管理员密码

```bash
# 删除现有管理员账户
psql "postgresql://palpo:password@localhost/palpo" -c "DELETE FROM webui_admin_credentials;"

# 创建新的管理员账户（密码: AdminTest123!）
curl -X POST http://localhost:8081/api/v1/auth/webui/setup \
  -H "Content-Type: application/json" \
  -d '{"password":"AdminTest123!"}'
```

### 终端 4: 运行 E2E 测试

```bash
# 测试 3.4.1: 管理员创建新用户
agent-browser open "http://localhost:8080"
agent-browser wait 3000
agent-browser snapshot -i

# 登录
agent-browser fill @e1 "admin"
agent-browser fill @e2 "AdminTest123!"
agent-browser click @e4
agent-browser wait 3000
agent-browser snapshot -i

# 处理可能的对话框
agent-browser find text "我明白了" click || true
agent-browser wait 2000
agent-browser snapshot -i

# 导航到用户管理页面
agent-browser find text "用户管理" click || agent-browser find text "用户" click
agent-browser wait 2000
agent-browser snapshot -i

# 点击创建用户
agent-browser find text "创建用户" click
agent-browser wait 1000
agent-browser snapshot -i

# 填写表单
agent-browser fill @e1 "testuser001"
agent-browser fill @e2 "Test User"
agent-browser fill @e3 "test@example.com"

# 检查用户名可用性
agent-browser find text "检查可用性" click
agent-browser wait 1000
agent-browser snapshot -i

# 生成密码
agent-browser find text "生成密码" click
agent-browser wait 500

# 提交表单
agent-browser find text "创建" click
agent-browser wait 2000
agent-browser snapshot -i

# 关闭浏览器
agent-browser close
```

## 验证结果

在每个 `snapshot -i` 后，检查输出：
- 登录后应该看到主界面
- 用户管理页面应该显示用户列表
- 创建用户后应该看到成功消息

## 清理

```bash
# 停止所有服务 (Ctrl+C 在各个终端)
# 或者
pkill -f "palpo-admin-server"
pkill -f "dx serve"
```

## 故障排查

### 问题 1: Admin Server 无法启动
- 检查 8081 端口是否被占用: `lsof -i :8081`
- 检查数据库连接: `psql "postgresql://palpo:password@localhost/palpo" -c "SELECT 1;"`

### 问题 2: Admin UI 无法启动
- 检查 8080 端口是否被占用: `lsof -i :8080`
- 检查 Dioxus 是否安装: `dx --version`

### 问题 3: 登录失败
- 确认管理员密码已创建:
  ```bash
  psql "postgresql://palpo:password@localhost/palpo" -c "SELECT username, created_at FROM webui_admin_credentials;"
  ```
- 如果没有记录，重新执行"重置管理员密码"步骤

### 问题 4: agent-browser 找不到元素
- 使用 `agent-browser snapshot -i` 查看当前页面元素
- 检查元素引用 (如 @e1, @e2) 是否正确
- 可能需要等待更长时间让页面加载完成
