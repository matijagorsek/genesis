// Genesis in System Settings. The page is QML; it reads and writes through the local Genesis daemon
// (127.0.0.1:11520), the same data the Genesis Settings page uses. Nothing here leaves the machine.
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
    }
};

K_PLUGIN_CLASS_WITH_JSON(KCMGenesis, "kcm_genesis.json")
#include "kcm.moc"
