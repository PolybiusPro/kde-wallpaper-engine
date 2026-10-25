// Parks the Wallpaper Engine desktop window ("WPE-Host", one window covering
// every monitor) off-screen and tells the org.local.wpebridge wallpaper plugin
// on each screen which window to stream, which part of it to show, and whether
// the engine has a wallpaper there at all (otherwise the plugin shows the
// screen's own Plasma wallpaper, copied over from its org.kde.image settings).
// The window must stay un-minimized or KWin stops its screencast.
//
// wpehost.exe lists the engine's render windows in the host's title, relative
// to the host: "WPE-Host|x,y,w,h;x,y,w,h". In the engine's clone layout it
// makes one window and mirrors it on every monitor, flagged "WPE-Host:clone|...".
var HOST = "WPE-Host";
var PARK_X = -20000;
var PARK_Y = -20000;
var hostUuid = "";
var hostCaption = HOST;
var sent = {};

// Bounding box of all screens. Wine lays the host window out over the same
// area, so a screen's position inside it is its offset from this box's corner.
function bounds() {
    var x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    workspace.screens.forEach(function (s) {
        var g = s.geometry;
        x0 = Math.min(x0, g.x);
        y0 = Math.min(y0, g.y);
        x1 = Math.max(x1, g.x + g.width);
        y1 = Math.max(y1, g.y + g.height);
    });
    return {x: x0, y: y0, width: x1 - x0, height: y1 - y0};
}

// Render windows from the host's title, as {x, y, w, h} in host coordinates.
function renderWindows() {
    var i = hostCaption.indexOf("|");
    if (i < 0) { return []; }
    return hostCaption.substring(i + 1).split(";").map(function (t) {
        var v = t.split(",").map(Number);
        return {x: v[0], y: v[1], w: v[2], h: v[3]};
    });
}

function cloneMode() {
    return hostCaption.indexOf(HOST + ":clone") === 0;
}

// The part of the screen's rect (host coordinates) that shows the engine's
// wallpaper: the window itself, or in clone mode the biggest centred area of
// the first window with the screen's aspect ratio (cover, like Plasma's Fill).
function cropFor(g, b, wins) {
    var crop = {x: g.x - b.x, y: g.y - b.y, w: g.width, h: g.height};
    if (cloneMode() && wins.length > 0) {
        var src = wins[0];
        var k = Math.min(src.w / g.width, src.h / g.height);
        crop.w = Math.round(g.width * k);
        crop.h = Math.round(g.height * k);
        crop.x = src.x + Math.round((src.w - crop.w) / 2);
        crop.y = src.y + Math.round((src.h - crop.h) / 2);
    }
    return crop;
}

// A screen has a wallpaper when a render window covers its centre.
function assigned(crop, wins) {
    var cx = crop.x + crop.w / 2;
    var cy = crop.y + crop.h / 2;
    return wins.some(function (r) {
        return cx >= r.x && cx < r.x + r.w && cy >= r.y && cy < r.y + r.h;
    });
}

function publish(screen, uuid, crop) {
    var key = uuid + JSON.stringify(crop);
    if (sent[screen] === key) { return; }
    sent[screen] = key;
    var js = "desktops().forEach(function (d) { if (d.screen === " + screen + ") {" +
        " d.currentConfigGroup = ['Wallpaper', 'org.local.wpebridge', 'General'];" +
        " d.writeConfig('WindowUuid', '" + uuid + "');" +
        " d.writeConfig('CropX', " + crop.x + "); d.writeConfig('CropY', " + crop.y + ");" +
        " d.writeConfig('CropW', " + crop.w + "); d.writeConfig('CropH', " + crop.h + ");" +
        " d.writeConfig('HostW', " + crop.hostW + "); d.writeConfig('HostH', " + crop.hostH + ");" +
        " d.writeConfig('Assigned', " + crop.assigned + ");" +
        // Copy the screen's own Plasma wallpaper, which Plasma keeps in the
        // org.kde.image group even while this plugin is the active one.
        " d.currentConfigGroup = ['Wallpaper', 'org.kde.image', 'General'];" +
        " var img = d.readConfig('Image'); var fill = d.readConfig('FillMode', 2);" +
        " d.currentConfigGroup = ['Wallpaper', 'org.local.wpebridge', 'General'];" +
        " d.writeConfig('FallbackImage', String(img)); d.writeConfig('FallbackFillMode', Number(fill)); } });";
    callDBus("org.kde.plasmashell", "/PlasmaShell", "org.kde.PlasmaShell", "evaluateScript", js);
}

function publishAll() {
    var b = bounds();
    var wins = renderWindows();
    workspace.screens.forEach(function (s, i) {
        var crop = cropFor(s.geometry, b, wins);
        crop.hostW = b.width;
        crop.hostH = b.height;
        crop.assigned = assigned(crop, wins);
        publish(i, hostUuid, crop);
    });
}

function isHost(w) {
    return w.caption.indexOf(HOST) === 0;
}

// Wallpaper Engine's own "other application fullscreen / maximized" settings
// cannot see native windows, so work out here which screens are covered and
// tell the wpe-bridge-playback helper, which applies the engine's setting.
// The state is one word per screen showing the engine, in screen order:
// "fullscreen", "maximized" or "none".
var playbackSent = "";

// Windows of the engine itself (its UI is a Wine window) never count as covering.
function engineWindow(w) {
    return isHost(w) || /^(wallpaper|wpehost|launcher)/i.test(String(w.resourceClass));
}

function covers(w, screen, skip) {
    if (w === skip || w.minimized || w.deleted || !w.normalWindow || engineWindow(w)) { return false; }
    if (w.desktops.length > 0 && w.desktops.indexOf(workspace.currentDesktop) < 0) { return false; }
    return w.output && w.output.name === screen.name;
}

function coverKind(screen, skip) {
    var kind = "none";
    workspace.windowList().forEach(function (w) {
        if (!covers(w, screen, skip)) { return; }
        if (w.fullScreen) {
            kind = "fullscreen";
        } else if (kind === "none") {
            // Maximized windows leave the panel's strip free, so compare with the work area.
            var area = workspace.clientArea(KWin.MaximizeArea, screen, workspace.currentDesktop);
            var g = w.frameGeometry;
            if (g.width >= area.width - 2 && g.height >= area.height - 2) { kind = "maximized"; }
        }
    });
    return kind;
}

function updatePlayback(skip) {
    var b = bounds();
    var wins = renderWindows();
    var state = [];
    workspace.screens.forEach(function (s) {
        if (assigned(cropFor(s.geometry, b, wins), wins)) { state.push(coverKind(s, skip)); }
    });
    var joined = state.join(",");
    if (joined === playbackSent) { return; }
    playbackSent = joined;
    callDBus("org.local.WpeBridge", "/Playback", "org.local.WpeBridge.Playback", "SetState", joined);
}

function park(w) {
    if (!isHost(w)) { return; }
    hostCaption = w.caption;
    w.noBorder = true;
    w.skipTaskbar = true;
    w.skipPager = true;
    w.skipSwitcher = true;
    w.desktops = [];
    if (w.fullScreen) { w.fullScreen = false; }
    // KWin clamps new windows to the work area, so force the exact size or the
    // streamed picture gets cropped from the wrong offsets.
    var b = bounds();
    var g = w.frameGeometry;
    if (g.x !== PARK_X || g.y !== PARK_Y || g.width !== b.width || g.height !== b.height) {
        w.frameGeometry = {x: PARK_X, y: PARK_Y, width: b.width, height: b.height};
    }
    hostUuid = String(w.internalId);
    publishAll();
    // The render windows changed (or the engine restarted): report again, even
    // if the covered state is the same, so a restarted helper catches up.
    playbackSent = "";
    updatePlayback();
}

function watch(w) {
    w.captionChanged.connect(function () { park(w); });
    w.frameGeometryChanged.connect(function () { park(w); updatePlayback(); });
    w.fullScreenChanged.connect(function () { park(w); updatePlayback(); });
    w.minimizedChanged.connect(function () { updatePlayback(); });
    w.desktopsChanged.connect(function () { updatePlayback(); });
    w.outputChanged.connect(function () { updatePlayback(); });
    park(w);
    updatePlayback();
}

workspace.windowAdded.connect(watch);
workspace.windowRemoved.connect(function (w) {
    if (isHost(w)) {
        hostUuid = "";
        hostCaption = HOST;
        publishAll();
    }
    updatePlayback(w);
});
workspace.currentDesktopChanged.connect(function () { updatePlayback(); });
if (workspace.screensChanged) {
    workspace.screensChanged.connect(function () {
        if (hostUuid !== "") { publishAll(); }
        updatePlayback();
    });
}
workspace.windowList().forEach(watch);
