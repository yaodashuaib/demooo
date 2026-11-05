#pragma once

#include "BridgeClient.h"

#include <QObject>
#include <QVariantList>

class ConfigViewModel : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool online MEMBER m_online NOTIFY statusChanged)
    Q_PROPERTY(QString serial MEMBER m_serial NOTIFY statusChanged)
    Q_PROPERTY(QVariantList agents MEMBER m_agents NOTIFY statusChanged)
    Q_PROPERTY(QString error MEMBER m_error NOTIFY statusChanged)
    Q_PROPERTY(int soundVolume MEMBER m_soundVolume NOTIFY soundLightChanged)
    Q_PROPERTY(bool soundEnabled MEMBER m_soundEnabled NOTIFY soundLightChanged)
    Q_PROPERTY(int lightBrightness MEMBER m_lightBrightness NOTIFY soundLightChanged)
    Q_PROPERTY(bool lightEnabled MEMBER m_lightEnabled NOTIFY soundLightChanged)
    Q_PROPERTY(int lightDurationMs MEMBER m_lightDurationMs NOTIFY soundLightChanged)
    Q_PROPERTY(int lightR MEMBER m_lightR NOTIFY soundLightChanged)
    Q_PROPERTY(int lightG MEMBER m_lightG NOTIFY soundLightChanged)
    Q_PROPERTY(int lightB MEMBER m_lightB NOTIFY soundLightChanged)
    Q_PROPERTY(bool workbuddyConfigured MEMBER m_workbuddyConfigured NOTIFY statusChanged)
    Q_PROPERTY(QString workbuddyCookieDraft MEMBER m_workbuddyCookieDraft NOTIFY workbuddyDraftChanged)
    Q_PROPERTY(QStringList allowlist MEMBER m_allowlist CONSTANT)
    Q_PROPERTY(QString bindId MEMBER m_bindId NOTIFY bindDraftChanged)
    Q_PROPERTY(QString bindName MEMBER m_bindName NOTIFY bindDraftChanged)
    Q_PROPERTY(int balanceRefreshSecs MEMBER m_balanceRefreshSecs NOTIFY statusChanged)

public:
    explicit ConfigViewModel(QObject *parent = nullptr);

    Q_INVOKABLE void refresh();
    Q_INVOKABLE void bindAgent();
    Q_INVOKABLE void unbindAgent(const QString &id);
    Q_INVOKABLE void setBalanceInterval();
    Q_INVOKABLE void applySoundLight(bool preview);
    Q_INVOKABLE void saveWorkbuddySession();
    Q_INVOKABLE void clearWorkbuddySession();

signals:
    void statusChanged();
    void soundLightChanged();
    void workbuddyDraftChanged();
    void bindDraftChanged();

private:
    void applyConfigJson(const QJsonObject &config);

    BridgeClient m_client;
    bool m_online = false;
    QString m_serial;
    QVariantList m_agents;
    QString m_error;
    int m_soundVolume = 80;
    bool m_soundEnabled = true;
    int m_lightBrightness = 80;
    bool m_lightEnabled = true;
    int m_lightDurationMs = 6000;
    int m_lightR = 255;
    int m_lightG = 128;
    int m_lightB = 0;
    bool m_workbuddyConfigured = false;
    QString m_workbuddyCookieDraft;
    QStringList m_allowlist;
    QString m_bindId;
    QString m_bindName;
    int m_balanceRefreshSecs = 180;
};
