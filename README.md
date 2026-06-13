# ritz

> fitz, reforged in Rust.

高性能 PDF 处理库（Rust + MuPDF），输出 API 兼容 PyMuPDF 的 Python 包。

## 环境准备

### 1. 安装编译工具链

```bash
# macOS
xcode-select --install
brew install make

# Linux (Debian/Ubuntu)
sudo apt install build-essential pkg-config
```

### 2. 添加 MuPDF 子模块

```bash
cd pdf-rs
git init
git submodule add https://github.com/ArtifexSoftware/mupdf.git vendor/mupdf
cd vendor/mupdf
git checkout 1.27.0        # 锁定版本
cd ../..
git add vendor/mupdf
```

### 3. 构建

```bash
cargo build                  # 自动编译 MuPDF + Rust
cargo test                   # 运行集成测试
```

## 架构

四层设计（借鉴 LiteParse）：

| crate | 职责 |
|---|---|
| `mupdf-sys` | FFI 绑定 + C 错误包装层（longjmp 隔离） |
| `mupdf` | 安全 RAII 封装、错误转换、坐标系转换 |
| `ritz` | 高层 Rust API + PyO3 Python 绑定（输出 `_ritz` cdylib） |

详见 `../plan_v2.md`。

## 命名

`import ritz` 而非 `import fitz`，避免与已安装的 PyMuPDF 命名空间冲突。
API 与 PyMuPDF 核心方法兼容，迁移时将 `import fitz` 改为 `import ritz` 即可。

## 许可证

AGPL-3.0（MuPDF 为 AGPL，本项目同步）
