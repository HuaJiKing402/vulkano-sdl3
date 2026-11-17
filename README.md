# Vulkano + SDL3 三角形模板

一个最小可运行模板：SDL3 负责窗口和事件，vulkano 0.35 负责 Vulkan 初始化、交换链、图形管线与绘制。渲染结果是一个粉红色三角形。

已在本机验证：Rust 1.92（windows-gnu）、Vulkan 1.4.357、AMD RX 5700 XT，运行 5 帧后自动退出，无校验错误。

## 目录结构

```text
Cargo.toml
build.rs                  # 为 MinGW 链接器添加 native-libs 搜索路径
src/main.rs               # 全部逻辑：窗口 → surface → 设备 → 管线 → 帧循环
shaders/tri.vert          # 顶点着色器源码
shaders/tri.frag          # 片段着色器源码
shaders/*.spv             # 用 glslc 预编译的 SPIR-V
native-libs/              # MinGW 下从 SDL3.dll 生成的导入库
```

## 环境要求

- Rust（本机是 `x86_64-pc-windows-gnu`）
- Vulkan SDK（提供 Loader，本项目通过 SDK 的 `SDL3.dll` / `SDL3.lib` 链接）
- `glslc`（只在修改着色器后需要）

本机 SDK 路径：`F:/source/VulkanSDK/1.4.357.0`。如果你装在其他位置，把下面命令里的路径换掉。

## 构建与运行

在本机（windows-gnu 工具链）需要三个环境变量：

```bash
export SDK=F:/source/VulkanSDK/1.4.357.0
export SHADERC_LIB_DIR="$SDK/Bin"   # 让 shaderc-sys 直接链接 SDK 的 DLL，跳过 cmake
export LIB="$SDK/Lib"               # SDL3 的 MSVC 导入库搜索路径（保留兼容）
export PATH="$SDK/Bin:$PATH"        # 运行时找到 SDL3.dll
cargo run
```

运行后会出现一个 800x600 窗口，按 Esc 或关闭窗口退出。

**无交互式自动退出**（用于 CI / 快速验证）：

```bash
TRIANGLE_MAX_FRAMES=5 cargo run
```

### 为什么需要 native-libs

本机 Rust 工具链是 GNU（MinGW），而 Vulkan SDK 只带 MSVC 格式的 `SDL3.lib`，链接器无法直接使用。仓库里的 `native-libs/libSDL3.dll.a` 是从 `SDL3.dll` 生成的 GNU 导入库，`build.rs` 会自动把它加入链接搜索路径。

如果你换了机器或 SDL3 版本，重新生成：

```bash
gendef - "$SDK/Bin/SDL3.dll" > native-libs/SDL3.def
dlltool -D SDL3.dll -d native-libs/SDL3.def -l native-libs/libSDL3.dll.a
```

如果你用 MSVC 工具链（`x86_64-pc-windows-msvc`），则不需要 `native-libs`，直接设置 `LIB` 和 `PATH` 即可。

## 修改着色器

着色器通过 `vulkano_shaders::shader!` 的 `bytes` 选项加载预编译 SPIR-V，这样构建时不需要 cmake / shaderc：

```bash
glslc shaders/tri.vert -o shaders/tri.vert.spv
glslc shaders/tri.frag -o shaders/tri.frag.spv
cargo build
```

## 代码要点

1. **窗口 → surface**：SDL3 创建带 `.vulkan()` 标记的窗口；`Surface::required_extensions(&window)` 查询所需实例扩展；因为 SDL3 的 `Window` 不是 `Send + Sync`，使用 `unsafe Surface::from_window_ref(instance, &window)`，窗口生命周期覆盖 surface。
2. **设备选择**：要求 `khr_swapchain`，队列族同时支持 GRAPHICS 和对该窗口的呈现。
3. **交换链**：`Swapchain::new` + 从交换链图像创建 `ImageView` 和 `Framebuffer`；窗口尺寸变化时重建。
4. **管线**：`shader!` 生成着色器模块，`PipelineLayout::new` + `GraphicsPipeline::new`，viewport 为动态状态，所以 resize 不需要重建管线。
5. **帧循环**：`acquire_next_image` → 录制 `begin_render_pass / set_viewport / bind_pipeline / bind_vertex_buffers / draw / end_render_pass` → `then_execute` → `then_swapchain_present` → `then_signal_fence_and_flush`；用 `GpuFuture` 串起帧间依赖。

## 版本

- `vulkano 0.35.2`
- `vulkano-shaders 0.35.0`
- `sdl3 0.22.1`（feature `raw-window-handle`）
- `ash 0.38.0`（vulkano 间接依赖）
