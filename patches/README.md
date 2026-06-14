# patches/ — MuPDF 补丁管理（plan_v1 §5.5）

本目录存放对 `vendor/mupdf` 子模块的本地修改。`crates/mupdf-sys/build.rs`
在每次编译前自动按字母顺序应用所有 `*.patch` 文件。

## 工作流

### 1. 修改子模块

```bash
cd vendor/mupdf
# 编辑文件...
# 例如：新增一个 extern 声明，或调整内部函数行为
```

### 2. 生成补丁

```bash
cd vendor/mupdf
git diff > ../../patches/0001-<short-name>.patch
git checkout .   # 回滚子模块（让 build.rs 重新应用）
```

编号约定：`0001-`, `0002-`, ...，保证应用顺序明确。

### 3. 验证补丁

```bash
# 测试能否干净应用
git -C vendor/mupdf apply --check patches/0001-xxx.patch

# 触发 build.rs 应用
cargo build -p mupdf-sys
# 应看到：warning: applied patch 0001-xxx.patch
```

### 4. 二次编译应是幂等的

```bash
cargo build -p mupdf-sys
# 无 "applied" 警告 = 已应用，正确跳过
```

## 设计原则（plan_v1 §5.5）

- **不修改 MuPDF 已有函数的行为** — 只在独立文件新增辅助函数
- **补丁文件命名**：`mupdf_ext_` 前缀给新增函数，`mupdf_safe_` 给错误包装
- **向上游贡献**：通用性强的扩展整理后尝试 PR 至 ArtifexSoftware/mupdf
- **季度同步**：合并上游 master，必要时重新适配补丁

## 当前补丁清单

| 补丁 | 内容 |
|------|------|
| `0001-fit_z-add-ritz-marker-comment.patch` | 示例：在 `fitz.h` 头部加一行注释，演示工作流（不影响行为）|

## 注意事项

- 补丁失败不会中断构建（只发警告），便于 CI 不被补丁冲突卡住
- 真实冲突需要本地排查：通常是子模块版本变了导致 diff context 不匹配
- 升级 MuPDF 子模块后，建议先清空 patches/ 重新生成
