// genesis-window: a plain Qt WebEngine window for Genesis surfaces (first-run wizard, maker workspace).
// No tabs, no address bar. Local URLs only.
//
//   genesis-window http://127.0.0.1:11520/          open the maker workspace
//   genesis-window --first-run                        open the wizard if first run is not complete, else exit
//   genesis-window --kiosk URL                        fullscreen

#include <QApplication>
#include <QFile>
#include <QMainWindow>
#include <QProcess>
#include <QTimer>
#include <QUrl>
#include <QWebEngineProfile>
#include <QWebEngineSettings>
#include <QWebEngineView>
#include <QtGlobal>
#include <QThread>
#include <QIcon>

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
    QString url = args.size() > 1 ? args.at(1) : QStringLiteral("http://127.0.0.1:11520/");

    if (firstRun) {
        if (QFile::exists("/var/lib/genesis/first-run-done"))
            return 0;
        url = QStringLiteral("http://127.0.0.1:11510/");
        kiosk = true;
    }
    if (!url.startsWith("http://127.0.0.1") && !url.startsWith("http://localhost"))
        return 2; // local surfaces only

    // wait briefly for the local service to come up (first login races the daemons)
    for (int i = 0; i < 60 && !reachable(url); ++i)
        QThread::sleep(1);

    auto *win = new QMainWindow;
    win->setWindowTitle("Genesis");
    win->setWindowIcon(QIcon::fromTheme("genesis"));
    auto *view = new QWebEngineView(win);
    view->settings()->setAttribute(QWebEngineSettings::LocalContentCanAccessRemoteUrls, false);
    view->settings()->setAttribute(QWebEngineSettings::JavascriptCanOpenWindows, false);
    view->load(QUrl(url));
    win->setCentralWidget(view);
    win->resize(1280, 820);
    if (kiosk) win->showFullScreen(); else win->show();

    // first-run: close automatically once setup is complete
    if (firstRun) {
        auto *t = new QTimer(win);
        QObject::connect(t, &QTimer::timeout, [&]() {
            if (QFile::exists("/var/lib/genesis/first-run-done")) {
                QProcess::startDetached("genesis-window", {"http://127.0.0.1:11520/"});
                app.quit();
            }
        });
        t->start(2000);
    }
    return app.exec();
}
