// genesis-window: a plain Qt WebEngine window for Genesis surfaces (first-run wizard, maker workspace).
// No tabs, no address bar. Local URLs only.
//
//   genesis-window http://127.0.0.1:11520/          open the maker workspace
//   genesis-window --first-run                        open the wizard if first run is not complete, else exit
//   genesis-window --kiosk URL                        fullscreen
//   genesis-window --palette [--selection TEXT] [--listen]   the one door: a small centered window for a request; --listen starts the mic at once

#include <QApplication>
#include <QFile>
#include <QMainWindow>
#include <QProcess>
#include <QTimer>
#include <QUrl>
#include <QUrlQuery>
#include <QWebEngineProfile>
#include <QWebEngineSettings>
#include <QWebEngineView>
#include <QWebEnginePage>
#include <QWebEngineUrlRequestInterceptor>
#include <QWebEngineUrlRequestInfo>
#include <QStandardPaths>
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
#include <QWebEnginePermission>
#endif
#include <QtGlobal>
#include <QThread>
#include <QIcon>
#include <QScreen>
#include <QGuiApplication>

// The maker's pages get their API token from this window, never from the daemon on a bare GET: a
// sandboxed shell with network access shares the loopback interface and could otherwise fetch the page,
// read the token out of it and answer its own permission cards. The token file lives under
// XDG_RUNTIME_DIR, which the sandbox does not see (it has a tmpfs over /run).
class TokenInterceptor : public QWebEngineUrlRequestInterceptor {
public:
    explicit TokenInterceptor(QByteArray t, int p, QObject *parent) : QWebEngineUrlRequestInterceptor(parent), token(std::move(t)), port(p) {}
    void interceptRequest(QWebEngineUrlRequestInfo &info) override {
        const QUrl u = info.requestUrl();
        if (!token.isEmpty() && u.host() == "127.0.0.1" && u.port() == port) info.setHttpHeader("X-Genesis-Token", token);
    }
private:
    QByteArray token;
    int port;
};

// Every account has its own maker daemon on its own port, written to the runtime directory. Menus, the
// panel and the scripts all say http://127.0.0.1:11520/ -- the maker's address as a name -- and this
// window turns it into this user's port, so none of them has to know the number.
static int agentdPort() {
    QFile f(qEnvironmentVariable("XDG_RUNTIME_DIR") + "/genesis/agentd.port");
    if (f.open(QIODevice::ReadOnly)) { bool ok = false; const int p = QString::fromUtf8(f.readAll()).trimmed().toInt(&ok); if (ok && p > 0) return p; }
    return 11520;
}
static QString toThisUser(QString url) {
    QUrl u(url);
    if (u.host() == "127.0.0.1" && u.port() == 11520) { u.setPort(agentdPort()); return u.toString(); }
    return url;
}

/// Dark or light, as the desktop has it: a web view inside a Qt app reports the Qt palette, which is not
/// the Plasma colour scheme, so it rendered light pages on a dark desktop. The pages take it from the URL.
static QString desktopTheme() {
    const QString home = qEnvironmentVariable("HOME");
    for (const QString &f : {home + "/.config/kdedefaults/kdeglobals", home + "/.config/kdeglobals"}) {
        QFile g(f);
        if (!g.open(QIODevice::ReadOnly | QIODevice::Text)) continue;
        const QString text = QString::fromUtf8(g.readAll());
        for (const QString &key : {QStringLiteral("ColorScheme="), QStringLiteral("LookAndFeelPackage=")}) {
            const int at = text.indexOf(key);
            if (at < 0) continue;
            const QString value = text.mid(at + key.size(), 60).section('\n', 0, 0).toLower();
            if (value.contains("dark")) return QStringLiteral("dark");
            if (!value.isEmpty()) return QStringLiteral("light");
        }
    }
    return QStringLiteral("dark");  // Genesis ships dark
}

static QByteArray agentdToken() {
    QString dir = qEnvironmentVariable("XDG_RUNTIME_DIR");
    if (dir.isEmpty()) dir = QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
    QFile f(dir + "/genesis/agentd.token");
    return f.open(QIODevice::ReadOnly) ? f.readAll().trimmed() : QByteArray();
}

static bool reachable(const QString &url) {
    QProcess p;
    p.start("curl", {"-fsS", "-m", "2", "-o", "/dev/null", url});
    p.waitForFinished(3000);
    return p.exitCode() == 0;
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    app.setApplicationName("Genesis");
    app.setDesktopFileName("org.genesis.workspace");

    QStringList args = app.arguments();
    bool kiosk = args.removeAll("--kiosk") > 0;
    bool firstRun = args.removeAll("--first-run") > 0;
    bool palette = args.removeAll("--palette") > 0;
    bool listen = args.removeAll("--listen") > 0;  // the palette listens at once (Meta+Shift+V)
    QString selection;
    int si = args.indexOf("--selection");
    if (si >= 0 && si + 1 < args.size()) { selection = args.at(si + 1); args.removeAt(si + 1); args.removeAt(si); }
    QString url = args.size() > 1 ? args.at(1) : QStringLiteral("http://127.0.0.1:11520/");

    if (firstRun) {
        if (QFile::exists("/var/lib/genesis/first-run-done"))
            return 0;
        url = QStringLiteral("http://127.0.0.1:11510/");
        kiosk = true;
    }
    if (palette) {
        url = QStringLiteral("http://127.0.0.1:11520/palette");
        if (!selection.isEmpty()) url += "?selection=" + QString::fromUtf8(QUrl::toPercentEncoding(selection));
        if (listen) url += (url.contains('?') ? "&" : "?") + QStringLiteral("listen=1");
    }
    if (!url.startsWith("http://127.0.0.1") && !url.startsWith("http://localhost"))
        return 2; // local surfaces only

    // wait briefly for the local service to come up (first login races the daemons); the port file is
    // read again on every try, because at first login the daemon may not have written it yet
    for (int i = 0; i < 60 && !reachable(url = toThisUser(url)); ++i)
        QThread::sleep(1);

    auto *win = new QMainWindow;
    win->setWindowTitle("Genesis");
    win->setWindowIcon(QIcon::fromTheme("genesis"));
    auto *view = new QWebEngineView(win);
    view->settings()->setAttribute(QWebEngineSettings::LocalContentCanAccessRemoteUrls, false);
    view->settings()->setAttribute(QWebEngineSettings::JavascriptCanOpenWindows, false);
    // local surfaces may use the microphone (voice input goes to whisper.cpp on this machine)
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
    QObject::connect(view->page(), &QWebEnginePage::permissionRequested, [](QWebEnginePermission p) {
        if (p.permissionType() == QWebEnginePermission::PermissionType::MediaAudioCapture && p.origin().host() == "127.0.0.1") p.grant(); else p.deny();
    });
#else
    QObject::connect(view->page(), &QWebEnginePage::featurePermissionRequested, [view](const QUrl &o, QWebEnginePage::Feature f) {
        view->page()->setFeaturePermission(o, f, (f == QWebEnginePage::MediaAudioCapture && o.host() == "127.0.0.1") ? QWebEnginePage::PermissionGrantedByUser : QWebEnginePage::PermissionDeniedByUser);
    });
#endif
    {   // hand the page the desktop's own answer
        QUrl u(url);
        QUrlQuery q(u.query());
        if (!q.hasQueryItem("theme")) { q.addQueryItem("theme", desktopTheme()); u.setQuery(q); url = u.toString(); }
    }
    view->page()->profile()->setUrlRequestInterceptor(new TokenInterceptor(agentdToken(), agentdPort(), win));
    view->load(QUrl(url));
    win->setCentralWidget(view);
    // a comfortable window, not a screen-filling one: 78% of the screen, capped, centered
    auto fitWindow = [win]() {
        QRect avail = QGuiApplication::primaryScreen() ? QGuiApplication::primaryScreen()->availableGeometry() : QRect(0, 0, 1280, 800);
        int w = qMin(1280, int(avail.width() * 0.78));
        int h = qMin(820, int(avail.height() * 0.78));
        win->resize(w, h);
        win->move(avail.x() + (avail.width() - w) / 2, avail.y() + (avail.height() - h) / 2);
    };
    fitWindow();
    if (palette) {
        // the palette is a small, frameless, centered window; once a request is sent it grows into the maker
        win->setWindowFlags(Qt::Dialog | Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint);
        win->resize(760, 330);
        QObject::connect(view, &QWebEngineView::urlChanged, [win, view, fitWindow](const QUrl &u) {
            if (u.fragment() == "close") { win->close(); return; }
            if (!u.path().startsWith("/palette")) {
                win->setWindowFlags(Qt::Window);
                fitWindow();
                win->show();
                view->setFocus();
            }
        });
    }
    // first run is maximized, not fullscreen: the panel stays reachable, because on a laptop the first
    // thing the wizard needs is Wi-Fi and the network tray must not sit behind it (audit, 17 Sep)
    if (kiosk && firstRun) win->showMaximized(); else if (kiosk) win->showFullScreen(); else win->show();

    // first-run: close automatically once setup is complete
    if (firstRun) {
        auto *t = new QTimer(win);
        QObject::connect(t, &QTimer::timeout, [&]() {
            if (QFile::exists("/var/lib/genesis/first-run-done")) {
                QString target = "http://127.0.0.1:11520/";
                QFile starter("/var/lib/genesis/first-run-starter");
                if (starter.open(QIODevice::ReadOnly)) {
                    QString p = QString::fromUtf8(starter.readAll()).trimmed();
                    if (!p.isEmpty()) target += "?prompt=" + QString::fromUtf8(QUrl::toPercentEncoding(p));
                }
                QProcess::startDetached("genesis-window", {target});
                app.quit();
            }
        });
        t->start(2000);
    }
    return app.exec();
}
