// Cursor export and cursor-free capture for the exact matching Hyprland ABI.
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <hyprland/src/pointer/PointerManager.hpp>
#include <hyprland/src/desktop/view/WLSurface.hpp>
#include <hyprland/src/protocols/core/Compositor.hpp>
#include <hyprland/src/protocols/core/Seat.hpp>
#include <hyprland/src/protocols/types/Buffer.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/render/OpenGL.hpp>
#include <hyprland/src/output/MonitorResources.hpp>
#include <hyprland/src/managers/screenshare/ScreenshareManager.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/helpers/math/Math.hpp>
#if __has_include(<hyprland/src/render/Context.hpp>)
#include <hyprland/src/render/Context.hpp>
#define LIGHTOS_HYPRLAND_RENDER_CONTEXT
#endif
#include <cmath>
#include <sstream>
#include <stdexcept>
#include <unordered_map>
#include <bit>
#include <dlfcn.h>

static HANDLE handle;
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
static SP<IPC::Socket1::SCommand> command;
#else
static SP<SHyprCtlCommand> command;
#endif
static CHyprSignalListener changed;
static CHyprSignalListener renderStage;
static uint64_t serial = 1;
static CFunctionHook *cursorHook = nullptr, *captureHook = nullptr, *mirrorHook = nullptr;
static unsigned captureDepth = 0;
static bool cleanCapture = false;
static bool snapshotQueued = false;

struct CleanFrame {
    PHLMONITORREF monitor;
    SP<Render::IFramebuffer> framebuffer;
    bool valid = false;
};
static std::unordered_map<Monitor::CMonitorResources*, CleanFrame> cleanFrames;

// Hyprland 0.56 copies its normal render target, including software cursors,
// into the screenshare texture. overlay_cursor=false only prevents a second
// cursor draw. Save actual pixels before that first draw; never reconstruct or
// erase cursor-shaped pixels from an already composited image.
class CleanFrameElement : public IPassElement {
    PHLMONITORREF monitor;
  public:
    explicit CleanFrameElement(PHLMONITOR value) : monitor(value) {}
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
    bool needsLiveBlur(Render::CRenderContext&) override { return false; }
    bool needsPrecomputeBlur(Render::CRenderContext&) override { return false; }
    bool undiscardable(Render::CRenderContext&) override { return true; }
#else
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool undiscardable() override { return true; }
#endif
    const char* passName() override { return "LightOSCursorFreeFrame"; }
    ePassElementType type() override { return EK_CUSTOM; }
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
    std::vector<UP<IPassElement>> draw(Render::CRenderContext& context) override {
        auto& renderData = context.m_data;
#else
    std::vector<UP<IPassElement>> draw() override {
        auto& renderData = g_pHyprRenderer->m_renderData;
#endif
        const auto output = monitor.lock();
        if (!output || g_pHyprRenderer->type() != Render::IHyprRenderer::RT_GL)
            return {};
        const auto source = renderData.currentFB;
        if (!source || !source->isAllocated())
            return {};
        std::erase_if(cleanFrames, [](const auto& entry) { return entry.second.monitor.expired(); });
        auto& frame = cleanFrames[output->resources().get()];
        frame.monitor = monitor;
        const bool initialize = !frame.valid || !frame.framebuffer || frame.framebuffer->m_size != source->m_size ||
                                frame.framebuffer->m_drmFormat != source->m_drmFormat;
        frame.valid = false;
        GLint read = 0, draw = 0, viewport[4] = {}, activeTexture = 0, texture = 0, texture0 = 0;
        glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &read);
        glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &draw);
        glGetIntegerv(GL_VIEWPORT, viewport);
        glGetIntegerv(GL_ACTIVE_TEXTURE, &activeTexture);
        glGetIntegerv(GL_TEXTURE_BINDING_2D, &texture);
        glActiveTexture(GL_TEXTURE0);
        glGetIntegerv(GL_TEXTURE_BINDING_2D, &texture0);
        glActiveTexture(activeTexture);
        const bool scissor = glIsEnabled(GL_SCISSOR_TEST);
        const auto restore = Render::CScopeGuard([&] {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
            Render::GL::g_pHyprOpenGL->setViewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            Render::GL::g_pHyprOpenGL->setCapStatus(GL_SCISSOR_TEST, scissor);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, texture0);
            glActiveTexture(activeTexture);
            glBindTexture(GL_TEXTURE_2D, texture);
        });
        if (!frame.framebuffer)
            frame.framebuffer = g_pHyprRenderer->createFB("LightOS cursor-free capture");
        if (!frame.framebuffer->alloc(source->m_size.x, source->m_size.y, source->m_drmFormat))
            return {};
        frame.framebuffer->setImageDescription(source->imageDescription());
        glBindFramebuffer(GL_READ_FRAMEBUFFER, static_cast<Render::GL::CGLFramebuffer*>(source.get())->getFBID());
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, static_cast<Render::GL::CGLFramebuffer*>(frame.framebuffer.get())->getFBID());
        Render::GL::g_pHyprOpenGL->setCapStatus(GL_SCISSOR_TEST, false);
        // Work buffers only contain this pass's repainted regions. Keep the
        // previous clean pixels elsewhere, especially on cursor-only frames.
        CRegion damage = initialize ? CRegion(0, 0, source->m_size.x, source->m_size.y) : renderData.damage;
        if (!initialize)
            damage.transform(Math::wlTransformToHyprutils(Math::invertTransform(output->m_transform)), output->m_transformedSize.x, output->m_transformedSize.y);
        damage.intersect(0, 0, source->m_size.x, source->m_size.y);
        for (const auto& box : damage.getRects())
            glBlitFramebuffer(box.x1, box.y1, box.x2, box.y2,
                              box.x1, box.y1, box.x2, box.y2,
                              GL_COLOR_BUFFER_BIT, GL_NEAREST);
        frame.valid = true;
        return {};
    }
};

#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
using CursorRender = void (*)(Pointer::CPointerManager*, Render::CRenderContext&, PHLMONITOR, const Time::steady_tp&,
                              CRegion&, std::optional<Vector2D>, bool, bool);
static void renderCursor(Pointer::CPointerManager* manager, Render::CRenderContext& context,
                         PHLMONITOR monitor, const Time::steady_tp& now,
                         CRegion& damage, std::optional<Vector2D> position, bool screencopy, bool force) {
#else
using CursorRender = void (*)(Pointer::CPointerManager*, PHLMONITOR, const Time::steady_tp&,
                              CRegion&, std::optional<Vector2D>, bool, bool);
static void renderCursor(Pointer::CPointerManager* manager, PHLMONITOR monitor, const Time::steady_tp& now,
                         CRegion& damage, std::optional<Vector2D> position, bool screencopy, bool force) {
#endif
    if (!screencopy && monitor && monitor->needsACopyFB()) {
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
        g_pHyprRenderer->addPassElement(context, makeUnique<CleanFrameElement>(monitor));
#else
        g_pHyprRenderer->addPassElement(makeUnique<CleanFrameElement>(monitor));
#endif
        snapshotQueued = true;
    }
    // A cursor-inclusive screenshot still needs exactly one pointer: the
    // replacement texture no longer contains the monitor's software cursor.
    reinterpret_cast<CursorRender>(cursorHook->m_original)(manager,
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
                                                          context,
#endif
                                                          monitor, now, damage, position,
                                                          screencopy, force || (screencopy && cleanCapture));
}

#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
static void renderCapture(Screenshare::CScreenshareFrame* frame, Render::CRenderContext& context) {
#else
static void renderCapture(Screenshare::CScreenshareFrame* frame) {
#endif
    const auto previous = cleanCapture;
    ++captureDepth;
    cleanCapture = false;
    const auto restore = Render::CScopeGuard([&] { --captureDepth; cleanCapture = previous; });
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
    reinterpret_cast<void (*)(Screenshare::CScreenshareFrame*, Render::CRenderContext&)>(captureHook->m_original)(frame, context);
#else
    reinterpret_cast<void (*)(Screenshare::CScreenshareFrame*)>(captureHook->m_original)(frame);
#endif
}

static SP<Render::ITexture> mirrorTexture(Monitor::CMonitorResources* resources) {
    if (captureDepth) {
        const auto found = cleanFrames.find(resources);
        cleanCapture = found != cleanFrames.end() && found->second.valid && !found->second.monitor.expired();
        // Until a clean frame exists, omit the image instead of leaking a
        // cursor-containing texture into an independent-cursor connection.
        return cleanCapture ? found->second.framebuffer->getTexture() : nullptr;
    }
    return reinterpret_cast<SP<Render::ITexture> (*)(Monitor::CMonitorResources*)>(mirrorHook->m_original)(resources);
}

template <typename Method> static void* methodAddress(Method method) {
    // Nonvirtual methods in the matching Itanium C++ ABI. Resolving the actual
    // method also works when Hyprland is launched through a private ELF loader;
    // searching /proc/self/exe would search ld-linux in that installation.
    struct Address { void* function; std::ptrdiff_t adjustment; };
    const auto address = std::bit_cast<Address>(method);
    if (address.adjustment != 0) throw std::runtime_error("Unsupported Hyprland method ABI");
    return address.function;
}

static void installCaptureHooks() {
    const auto install = [](const char* name, const void* address, const void* replacement) {
        auto* hook = address ? HyprlandAPI::createFunctionHook(handle, address, replacement) : nullptr;
        if (!hook || !hook->hook()) throw std::runtime_error(std::string("Cannot install LightOS capture hook: ") + name);
        return hook;
    };
    try {
        mirrorHook = install("getMirrorTexture", methodAddress(&Monitor::CMonitorResources::getMirrorTexture), reinterpret_cast<const void*>(mirrorTexture));
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
        captureHook = install("renderMonitor", dlsym(RTLD_DEFAULT, "_ZN11Screenshare17CScreenshareFrame13renderMonitorERN6Render14CRenderContextE"), reinterpret_cast<const void*>(renderCapture));
#else
        captureHook = install("renderMonitor", dlsym(RTLD_DEFAULT, "_ZN11Screenshare17CScreenshareFrame13renderMonitorEv"), reinterpret_cast<const void*>(renderCapture));
#endif
        cursorHook = install("renderSoftwareCursorsFor", methodAddress(&Pointer::CPointerManager::renderSoftwareCursorsFor), reinterpret_cast<const void*>(renderCursor));
    } catch (...) {
        for (auto* hook : {cursorHook, captureHook, mirrorHook})
            if (hook) HyprlandAPI::removeFunctionHook(handle, hook);
        cursorHook = captureHook = mirrorHook = nullptr;
        throw;
    }
}

static std::string quoted(const std::string& value) {
    static constexpr char hex[] = "0123456789abcdef";
    std::string result = "\"";
    for (unsigned char c : value) {
        if (c == '"' || c == '\\') { result += '\\'; result += c; }
        else if (c < 32) { result += "\\u00"; result += hex[c >> 4]; result += hex[c & 15]; }
        else result += c;
    }
    return result + '"';
}
static std::string snapshot(std::string request) {
    try {
        uint64_t previous = 0;
        std::string name;
        std::istringstream(request) >> name >> previous;
        const auto& image = Pointer::mgr()->currentCursorImage();
        const auto position = Pointer::mgr()->position();
        SP<Aquamarine::IBuffer> buffer = image.pBuffer;
        if (image.surface && image.surface->resource())
            buffer = image.surface->resource()->m_current.buffer.m_buffer;
        auto texture = Pointer::mgr()->getCurrentCursorTexture();
        const auto width = texture ? static_cast<int>(texture->m_size.x) : buffer ? static_cast<int>(buffer->size.x) : 0;
        const auto height = texture ? static_cast<int>(texture->m_size.y) : buffer ? static_cast<int>(buffer->size.y) : 0;
        if (!std::isfinite(image.scale) || image.scale <= 0 || image.scale > 8 ||
            !std::isfinite(position.x) || !std::isfinite(position.y) ||
            !std::isfinite(image.hotspot.x) || !std::isfinite(image.hotspot.y))
            throw std::runtime_error("invalid cursor geometry");
        std::ostringstream result;
        result.imbue(std::locale::classic());
        result << "{\"version\":1,\"cursor_free_capture\":true,\"serial\":" << serial << ",\"available\":true,\"visible\":"
            << ((image.pBuffer || image.surface) && g_pHyprRenderer->shouldRenderCursor() ? "true" : "false")
            << ",\"x\":" << position.x << ",\"y\":" << position.y
            << ",\"hotspot_x\":" << image.hotspot.x << ",\"hotspot_y\":" << image.hotspot.y
            << ",\"scale\":" << image.scale << ",\"width\":" << width << ",\"height\":" << height
            << ",\"outputs\":[";
        bool first = true;
        for (const auto& output : State::monitorState()->monitors()) {
            if (!first) result << ',';
            first = false;
            result << "{\"name\":" << quoted(output->m_name)
                << ",\"x\":" << output->m_position.x << ",\"y\":" << output->m_position.y
                << ",\"width\":" << output->m_size.x << ",\"height\":" << output->m_size.y << '}';
        }
        result << ']';
        if (buffer || texture) {
            if (width <= 0 || height <= 0 || width > 512 || height > 512)
                throw std::runtime_error("unsupported cursor size");
            const uint8_t* pixels = nullptr;
            size_t stride = 0;
            uint32_t format = 0;
            SP<Aquamarine::IBuffer> accessed;
            struct Access { SP<Aquamarine::IBuffer>& buffer; ~Access() { if (buffer) buffer->endDataPtr(); } } access{accessed};
            if (buffer) {
                auto attrs = buffer->shm();
                if (attrs.success && attrs.stride >= width * 4 && attrs.stride <= 65536 &&
                    (buffer->caps() & Aquamarine::BUFFER_CAPABILITY_DATAPTR)) {
                    auto data = buffer->beginDataPtr(1);
                    accessed = buffer;
                    pixels = std::get<0>(data); format = std::get<1>(data); stride = attrs.stride;
                }
            }
            if (!pixels && image.surface && image.surface->resource()) {
                const auto& cached = CCursorSurfaceRole::cursorPixelData(image.surface->resource());
                if (!cached.empty() && cached.size() % height == 0) {
                    pixels = cached.data(); stride = cached.size() / height;
                    format = texture ? texture->m_drmFormat : 0;
                }
            }
            if (!pixels && texture && !texture->dataCopy().empty() && texture->dataCopy().size() % height == 0) {
                pixels = texture->dataCopy().data();
                stride = texture->dataCopy().size() / height;
                format = texture->m_drmFormat;
            }
            if (!pixels || stride < static_cast<size_t>(width) * 4 || stride > 65536)
                throw std::runtime_error("cursor pixels unavailable");
            result << ",\"format\":" << format;
            if (previous != serial) {
                static constexpr char digits[] = "0123456789abcdef";
                std::string hex;
                hex.reserve(static_cast<size_t>(width) * height * 8);
                for (int y = 0; y < height; ++y) {
                    const auto row = pixels + static_cast<size_t>(y) * stride;
                    for (int x = 0; x < width * 4; ++x) {
                        hex.push_back(digits[row[x] >> 4]);
                        hex.push_back(digits[row[x] & 15]);
                    }
                }
                result << ",\"pixels\":\"" << hex << '\"';
            }
        }
        result << '}';
        return result.str();
    } catch (const std::exception& error) { return "{\"version\":1,\"cursor_free_capture\":true,\"available\":false,\"reason\":" + quoted(error.what()) + "}"; }
    catch (...) { return R"({"version":1,"cursor_free_capture":true,"available":false})"; }
}

APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }
APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE plugin) {
    if (std::string(__hyprland_api_get_hash()) != __hyprland_api_get_client_hash())
        throw std::runtime_error("LightOS cursor adapter requires matching Hyprland headers");
    handle = plugin;
    installCaptureHooks();
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
    renderStage = Event::bus()->m_events.render.stage.listen([](Event::SRenderStageEvent event) {
        if (!event.context) return;
        auto& context = event.context->get();
        const auto stage = event.stage;
        const auto monitor = event.monitor;
#else
    renderStage = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) {
        const auto monitor = g_pHyprRenderer->m_renderData.pMonitor.lock();
#endif
        if (stage == RENDER_BEGIN) snapshotQueued = false;
        // Hidden cursors skip renderSoftwareCursorsFor entirely. Their desktop
        // content must still update rather than reusing the last visible image.
        if (stage == RENDER_LAST_MOMENT && !snapshotQueued && monitor && monitor->needsACopyFB())
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
            g_pHyprRenderer->addPassElement(context, makeUnique<CleanFrameElement>(monitor));
#else
            g_pHyprRenderer->addPassElement(makeUnique<CleanFrameElement>(monitor));
#endif
    });
    changed = Pointer::mgr()->m_events.cursorChanged.listen([] { if (++serial == 0) serial = 1; });
    // Old observers registered "lightoscursor" as a prefix command. Use a
    // distinct name so an already loaded v1 cannot intercept the capability probe.
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
    command = HyprlandAPI::registerHyprCtlCommand(handle, IPC::Socket1::SCommand{
        "lightosindependentcursor", IPC::Socket1::COMMAND_MATCH_PREFIX,
        [](const IPC::Socket1::SRequest& request) { return IPC::Socket1::SResponse(snapshot(request.command)); }
    });
#else
    command = HyprlandAPI::registerHyprCtlCommand(handle, SHyprCtlCommand{
        "lightosindependentcursor", false,
        [](eHyprCtlOutputFormat, std::string request) { return snapshot(request); }
    });
#endif
    if (!command) throw std::runtime_error("Cannot register LightOS cursor observer");
    return {"lightos-cursor-v2", "Independent cursor and cursor-free screenshare", "LightOS", "2"};
}
APICALL EXPORT void PLUGIN_EXIT() {
#ifdef LIGHTOS_HYPRLAND_RENDER_CONTEXT
    g_pHyprRenderer->context().m_pass.removeAllOfType("LightOSCursorFreeFrame");
#else
    g_pHyprRenderer->m_renderPass.removeAllOfType("LightOSCursorFreeFrame");
#endif
    renderStage.reset();
    changed.reset();
    if (command) HyprlandAPI::unregisterHyprCtlCommand(handle, command);
    command.reset();
    for (auto* hook : {cursorHook, captureHook, mirrorHook})
        if (hook) HyprlandAPI::removeFunctionHook(handle, hook);
    cursorHook = captureHook = mirrorHook = nullptr;
    cleanFrames.clear();
}
