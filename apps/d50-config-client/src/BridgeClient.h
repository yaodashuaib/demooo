#pragma once

#include <QJsonObject>
#include <QObject>
#include <QString>

/// 经命名管道与 agent-bridge-service 通话的最小 NDJSON 客户端（Windows）。
class BridgeClient : public QObject
{
    Q_OBJECT
public:
    explicit BridgeClient(QObject *parent = nullptr);

    Q_INVOKABLE QJsonObject call(const QJsonObject &request, int timeoutMs = 8000);

    void setPipeName(const QString &pipe);

private:
    QString m_pipe;
};
