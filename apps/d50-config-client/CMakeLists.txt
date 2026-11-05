cmake_minimum_required(VERSION 3.21)
project(d50-config-client VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui Qml Quick Network)

qt_add_executable(d50-config-client
    src/main.cpp
    src/BridgeClient.cpp
    src/BridgeClient.h
    src/ConfigViewModel.cpp
    src/ConfigViewModel.h
)

qt_add_qml_module(d50-config-client
    URI D50Config
    VERSION 1.0
    QML_FILES
        qml/Main.qml
        qml/SoundLightPage.qml
        qml/WorkbuddySessionPage.qml
)

target_link_libraries(d50-config-client PRIVATE
    Qt6::Core
    Qt6::Gui
    Qt6::Qml
    Qt6::Quick
    Qt6::Network
)

install(TARGETS d50-config-client RUNTIME DESTINATION bin)
