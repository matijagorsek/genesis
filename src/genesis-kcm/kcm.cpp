// Genesis in System Settings. The page is QML; it reads and writes through the local Genesis daemon
// (127.0.0.1, this user's port), the same data the Genesis Settings page uses. Nothing here leaves the machine.
#include <KPluginFactory>
#include <KQuickConfigModule>
#include <QFile>
#include <QQmlContext>
#include <QQmlEngine>
#include <QStandardPaths>

class KCMGenesis : public KQuickConfigModule
{
    Q_OBJECT
public:
    KCMGenesis(QObject *parent, const KPluginMetaData &data)
        : KQuickConfigModule(parent, data)
    {
        setButtons(NoAdditionalButton);
        // the local daemon requires its per-boot token on every state-changing call (0600 under XDG_RUNTIME_DIR)
        QString token;
        QFile f(QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation) + QStringLiteral("/genesis/agentd.token"));
        if (f.open(QIODevice::ReadOnly)) token = QString::fromUtf8(f.readAll()).trimmed();
        engine()->rootContext()->setContextProperty(QStringLiteral("genesisToken"), token);
        // every account has its own daemon on its own port, written next to the token
        int port = 11520;
        QFile pf(QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation) + QStringLiteral("/genesis/agentd.port"));
        if (pf.open(QIODevice::ReadOnly)) { bool ok = false; const int p = QString::fromUtf8(pf.readAll()).trimmed().toInt(&ok); if (ok && p > 0) port = p; }
        engine()->rootContext()->setContextProperty(QStringLiteral("genesisPort"), port);
    }
};

K_PLUGIN_CLASS_WITH_JSON(KCMGenesis, "kcm_genesis.json")
#include "kcm.moc"
