#include "BridgeClient.h"

#include <QJsonDocument>
#include <QLocalSocket>

BridgeClient::BridgeClient(QObject *parent)
    : QObject(parent)
    , m_pipe(QStringLiteral(R"(\\.\pipe\ulanzi-d50-bridge.v1)"))
{
}

void BridgeClient::setPipeName(const QString &pipe)
{
    m_pipe = pipe;
}

QJsonObject BridgeClient::call(const QJsonObject &request, int timeoutMs)
{
    QLocalSocket socket;
    // QLocalSocket 在 Windows 上对 \\.\pipe\name 使用 name 部分。
    QString serverName = m_pipe;
    const QString prefix = QStringLiteral(R"(\\.\pipe\)");
    if (serverName.startsWith(prefix)) {
        serverName = serverName.mid(prefix.size());
    }
    socket.connectToServer(serverName);
    if (!socket.waitForConnected(timeoutMs)) {
        return QJsonObject{
            {QStringLiteral("result"), QStringLiteral("error")},
            {QStringLiteral("code"), QStringLiteral("SERVICE_UNAVAILABLE")},
            {QStringLiteral("message"), socket.errorString()},
        };
    }

    const QByteArray line = QJsonDocument(request).toJson(QJsonDocument::Compact) + '\n';
    if (socket.write(line) < 0 || !socket.waitForBytesWritten(timeoutMs)) {
        return QJsonObject{
            {QStringLiteral("result"), QStringLiteral("error")},
            {QStringLiteral("code"), QStringLiteral("IPC_IO")},
            {QStringLiteral("message"), socket.errorString()},
        };
    }

    QByteArray buffer;
    while (socket.waitForReadyRead(timeoutMs)) {
        buffer += socket.readAll();
        const int nl = buffer.indexOf('\n');
        if (nl >= 0) {
            const QJsonDocument doc = QJsonDocument::fromJson(buffer.left(nl));
            return doc.object();
        }
    }
    return QJsonObject{
        {QStringLiteral("result"), QStringLiteral("error")},
        {QStringLiteral("code"), QStringLiteral("IPC_TIMEOUT")},
        {QStringLiteral("message"), QStringLiteral("no response")},
    };
}
