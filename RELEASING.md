# ritz 发布手册

> 每次发版到 PyPI 的标准流程。

## 前置条件（一次性）

### 1. PyPI 账号

- 注册 https://pypi.org/account/register/
- 启用 2FA

### 2. TestPyPI 账号（用于发版前演练）

- 同样在 https://test.pypi.org 注册并启用 2FA

### 3. 配置 OIDC 可信发布（Trusted Publishing）

本项目用 **PyPI Trusted Publishing（OIDC）**，无需 API token secret。配置步骤：

**PyPI 正式环境**：
1. https://pypi.org/manage/account/publishing/ → Add a new publisher
2. Publisher type: **GitHub Actions**
3. 填三个字段：
   - PyPI Project Name: `ritz`（首次发版前项目不存在，选 "Reserve new project name"）
   - Owner: `justcodew`
   - Repository name: `ritz`
   - Workflow name: `release.yml`（对应 `.github/workflows/release.yml`）
   - Environment name: `pypi`（必须与 workflow 里 `environment:` 一致）

**TestPyPI**：同流程在 https://test.pypi.org/manage/account/publishing/，environment 填 `testpypi`。

> 关键不变量：**workflow 文件名 + environment 名 + repo** 三元组必须 PyPI 那边和 workflow 里完全一致，否则 OIDC token 会被拒。
> 工作流加 `permissions: id-token: write` 让 GitHub 发 OIDC token；不需要 `PYPI_API_TOKEN` secret。

## 发版流程

### 1. 确认要发版的版本号

```bash
grep '^version' Cargo.toml  # 当前版本
```

发版类型：
- patch（0.3.0 → 0.3.1）：bug fix
- minor（0.3.0 → 0.4.0）：新 API
- major（0.3.0 → 1.0.0）：不兼容变更

### 2. 改版本号（单一来源）

**只改 `Cargo.toml` 一处**，其他 4 处自动跟随：

```bash
sed -i '' 's/^version = "0.3.0"/version = "0.3.1"/' Cargo.toml  # macOS
# Linux: sed -i 's/.../' Cargo.toml
```

检查跟随：
```bash
grep -rn 'version.workspace\|version = "0.3' crates/*/Cargo.toml pyproject.toml
```

### 3. 更新 CHANGELOG

把 `[Unreleased]` 的内容移到新的版本节，日期填今天：

```markdown
## [0.3.1] - 2026-06-15

- Fixed: ...
```

### 4. 本地验证

```bash
source ~/.cargo/env && source .venv/bin/activate
maturin develop --release
python -c "import ritz; print(ritz.__version__)"  # 应输出 0.3.1
pytest python/tests/ -q  # 75 个必须全绿
```

### 5. 提交 + push

```bash
git add Cargo.toml CHANGELOG.md docx/  # 视改动而定
git commit -m "release: v0.3.1"
git push origin master
```

### 6. TestPyPI 演练（推荐）

```bash
git checkout -b test-release
git push origin test-release
```

GitHub Actions 会触发 `release.yml` 的 `testpypi` job。等 ~15 分钟构建完成。

验证安装：
```bash
python -m venv /tmp/testenv && source /tmp/testenv/bin/activate
pip install -i https://test.pypi.org/simple/ ritz
python -c "import ritz; d=ritz.open('crates/mupdf/tests/samples/sample.pdf'); print(len(d))"
```

### 7. 正式发布

```bash
git checkout master
git tag v0.3.1
git push origin v0.3.1
```

GitHub Actions 会触发 `release.yml` 的 `publish` job，自动构建 macOS arm64/x86_64 + Linux x86_64 wheel 并推到 PyPI。

监控：repo → Actions → release workflow，等绿灯。

### 8. 验证 PyPI

```bash
python -m venv /tmp/prodenv && source /tmp/prodenv/bin/activate
pip install ritz
python -c "import ritz; print(ritz.__version__)"
```

如果 PyPI 还没同步（CDN 慢），等几分钟再试。

### 9. 清理 TestPyPI 分支

```bash
git branch -D test-release
git push origin --delete test-release  # 可选
```

## 紧急回滚

PyPI 不允许重新上传同版本号。如果发布有严重 bug：

1. **立即发 patch 版本**（0.3.1 → 0.3.2），修 bug + 重走流程
2. 在 PyPI 项目页点 "Yank" 让坏版本不在默认 `pip install` 中出现（但仍能 `pip install ritz==0.3.1` 显式安装）

## 平台覆盖矩阵

当前发版的平台（在 `release.yml` matrix 中定义）。使用 abi3 wheel（`cp39-abi3`），
单个 wheel 覆盖 Python 3.9-3.13，无需按 Python 版本分别构建。

| 平台 | wheel 名 | 状态 |
|------|---------|------|
| Linux x86_64 | `ritz-X.Y.Z-cp39-abi3-manylinux_2_17_x86_64.whl` | ✅ |
| Linux aarch64 | `ritz-X.Y.Z-cp39-abi3-manylinux_2_28_aarch64.whl` | ✅ |
| macOS arm64 | `ritz-X.Y.Z-cp39-abi3-macosx_11_0_arm64.whl` | ✅ |
| macOS x86_64 | `ritz-X.Y.Z-cp39-abi3-macosx_10_12_x86_64.whl` | ✅（从 arm64 交叉编译） |
| Windows x86_64 | `ritz-X.Y.Z-cp39-abi3-win_amd64.whl` | ✅（MSBuild 编译 MuPDF） |

## 常见问题

### Q: wheel 大小多少？
A: ~29MB。PyPI 单文件默认上限是 100MB，无问题。

### Q: MuPDF 编译失败怎么办？
A: CI 里看哪一步失败。常见原因：
- 子模块没拉全：确认 `submodules: recursive`
- 系统缺 build deps：在 CI step 加 `apt-get install`
- patches/ 冲突：看 build log warning，按 onboarding §6.7 排查

### Q: 用户安装后 ImportError？
A: 检查 `module-name = "ritz._ritz"`（pyproject.toml）和 `from ._ritz import *`（python/ritz/__init__.py）是否一致。同 engine 不同 Python 版本要分别构建 wheel（maturin-action 默认会处理）。

### Q: AGPL 影响发布吗？
A: 不影响发布到 PyPI，但用户在闭源商业产品中用 ritz 需要买 MuPDF commercial license。这点要写在 README 显眼处。
