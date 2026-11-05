#include "ConfigViewModel.h"

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    QCoreApplication::setOrganizationName(QStringLiteral("Ulanzi"));
    QCoreApplication::setApplicationName(QStringLiteral("D50ConfigClient"));

    ConfigViewModel viewModel;
    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty(QStringLiteral("configVm"), &viewModel);
    QObject::connect(
        &engine,
        &QQmlApplicationEngine::objectCreationFailed,
        &app,
        []() { QCoreApplication::exit(-1); },
        Qt::QueuedConnection);
    engine.loadFromModule(QStringLiteral("D50Config"), QStringLiteral("Main"));
    viewModel.refresh();
    return app.exec();
}
