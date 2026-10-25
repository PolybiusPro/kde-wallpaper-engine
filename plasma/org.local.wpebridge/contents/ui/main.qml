import QtQuick
import org.kde.plasma.plasmoid
import org.kde.plasma.wallpapers.image as Wallpaper
import org.kde.pipewire as PipeWire
import org.kde.taskmanager as TaskManager

// Streams the Wallpaper Engine desktop window, which spans every monitor, and
// shows this screen's part of it. The wpe-bridge KWin script keeps WindowUuid
// pointed at that window, parks it off-screen and sets the crop rectangle and
// whether the engine has a wallpaper on this screen, plus the screen's own Plasma
// wallpaper (copied from its org.kde.image settings). When the engine has none (or
// is not running) the screen shows that wallpaper instead.
WallpaperItem {
    id: root

    readonly property var cfg: root.configuration
    readonly property bool cropped: cfg.HostW > 0 && cfg.CropW > 0
    // Logical pixels per host pixel, in case this screen is scaled.
    readonly property real k: cropped ? width / cfg.CropW : 1
    readonly property bool live: cfg.WindowUuid !== "" && cfg.Assigned

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    // Resolves a wallpaper folder (e.g. .../Coast/) to the image that fits this screen.
    Wallpaper.MediaProxy {
        id: media
        source: root.live ? "" : root.cfg.FallbackImage
        targetSize: Qt.size(root.width, root.height)
    }

    Image {
        anchors.fill: parent
        // Not loaded while the engine is showing something here.
        source: root.live ? "" : media.modelImage
        sourceSize: Qt.size(root.width, root.height)
        // Same numbering as Plasma's FillMode: 0 stretch, 1 fit, 2 crop, 6 pad.
        fillMode: root.cfg.FallbackFillMode === 0 ? Image.Stretch
                : root.cfg.FallbackFillMode === 1 ? Image.PreserveAspectFit
                : Image.PreserveAspectCrop
        asynchronous: true
    }

    Item {
        anchors.fill: parent
        clip: true
        visible: root.live

        TaskManager.ScreencastingRequest {
            id: request
            uuid: root.cfg.WindowUuid
        }

        PipeWire.PipeWireSourceItem {
            x: root.cropped ? -root.cfg.CropX * root.k : 0
            y: root.cropped ? -root.cfg.CropY * root.k : 0
            width: root.cropped ? root.cfg.HostW * root.k : parent.width
            height: root.cropped ? root.cfg.HostH * root.k : parent.height
            nodeId: request.nodeId
            visible: nodeId > 0
        }
    }
}
