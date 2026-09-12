// Genesis default layout: a centered, floating dock at the bottom. Launcher with the Genesis mark,
// the maker one click away, pinned everyday apps, tray and clock. Nothing else on the desktop.
var plasma = getApiVersion(1);
var layoutDesktop = new Desktop();
layoutDesktop.wallpaperPlugin = "org.kde.image";
layoutDesktop.currentConfigGroup = ["Wallpaper", "org.kde.image", "General"];
layoutDesktop.writeConfig("Image", "file:///usr/share/wallpapers/Genesis/");

var panel = new Panel();
panel.location = "bottom";
panel.alignment = "center";
panel.lengthMode = "fit";
panel.floating = true;
panel.hiding = "none";
panel.height = 2 * Math.floor(gridUnit * 2.9 / 2);

var kickoff = panel.addWidget("org.kde.plasma.kickoff");
kickoff.currentConfigGroup = ["General"];
kickoff.writeConfig("icon", "genesis");
kickoff.writeConfig("alphaSort", true);

var maker = panel.addWidget("org.kde.plasma.icon");
maker.currentConfigGroup = ["General"];
maker.writeConfig("url", "file:///usr/share/applications/org.genesis.workspace.desktop");

var tasks = panel.addWidget("org.kde.plasma.icontasks");
tasks.currentConfigGroup = ["General"];
tasks.writeConfig("launchers", ["applications:firefox.desktop", "applications:org.kde.dolphin.desktop", "applications:org.kde.konsole.desktop", "applications:org.kde.discover.desktop", "applications:systemsettings.desktop"]);
tasks.writeConfig("showOnlyCurrentScreen", false);

panel.addWidget("org.kde.plasma.marginsseparator");
panel.addWidget("org.kde.plasma.systemtray");
var clock = panel.addWidget("org.kde.plasma.digitalclock");
clock.currentConfigGroup = ["Appearance"];
clock.writeConfig("showDate", false);
panel.addWidget("org.kde.plasma.showdesktop");
