// Read-only cursor export for Hyprland builds whose cursor capture protocol
// does not expose both native-buffer and wl_surface cursors correctly.
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <hyprland/src/pointer/PointerManager.hpp>
#include <hyprland/src/desktop/view/WLSurface.hpp>
#include <hyprland/src/protocols/core/Compositor.hpp>
#include <hyprland/src/protocols/core/Seat.hpp>
#include <hyprland/src/protocols/types/Buffer.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <cmath>
#include <sstream>
#include <stdexcept>

static HANDLE handle;
static SP<SHyprCtlCommand> command;
static CHyprSignalListener changed;
static uint64_t serial = 1;

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
static std::string snapshot(eHyprCtlOutputFormat, std::string request) {
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
        result << "{\"version\":1,\"serial\":" << serial << ",\"available\":true,\"visible\":"
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
    } catch (const std::exception& error) { return "{\"version\":1,\"available\":false,\"reason\":" + quoted(error.what()) + "}"; }
    catch (...) { return R"({"version":1,"available":false})"; }
}

APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }
APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE plugin) {
    if (std::string(__hyprland_api_get_hash()) != __hyprland_api_get_client_hash())
        throw std::runtime_error("LightOS cursor adapter requires matching Hyprland headers");
    handle = plugin;
    changed = Pointer::mgr()->m_events.cursorChanged.listen([] { if (++serial == 0) serial = 1; });
    command = HyprlandAPI::registerHyprCtlCommand(handle, SHyprCtlCommand{"lightoscursor", false, snapshot});
    if (!command) throw std::runtime_error("Cannot register LightOS cursor observer");
    return {"lightos-cursor", "Read-only cursor image, hotspot, scale and position", "LightOS", "1"};
}
APICALL EXPORT void PLUGIN_EXIT() {
    changed.reset();
    if (command) HyprlandAPI::unregisterHyprCtlCommand(handle, command);
    command.reset();
}
