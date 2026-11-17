// Qt Widgets compatibility target: a window with a label, a line edit and a button (no moc needed).
// Typing a name and pressing the button (or Enter) greets it; the result is also printed for the serial log.
#include <QApplication>
#include <QLabel>
#include <QLineEdit>
#include <QPushButton>
#include <QVBoxLayout>
#include <QWidget>
#include <cstdio>

int main(int argc, char **argv)
{
    QApplication app(argc, argv);
    QWidget w;
    w.setWindowTitle("LoOS Qt Widgets");
    auto *label = new QLabel("Hello from Qt on LoOS");
    auto *edit = new QLineEdit;
    edit->setPlaceholderText("Your name");
    auto *button = new QPushButton("Greet");
    auto *box = new QVBoxLayout(&w);
    box->addWidget(label);
    box->addWidget(edit);
    box->addWidget(button);
    auto greet = [&] {
        QString t = "Hello, " + (edit->text().isEmpty() ? QString("world") : edit->text()) + "!";
        label->setText(t);
        std::printf("qt: %s\n", t.toUtf8().constData());
        std::fflush(stdout);
    };
    QObject::connect(button, &QPushButton::clicked, greet);
    QObject::connect(edit, &QLineEdit::returnPressed, greet);
    w.resize(420, 180);
    w.show();
    std::printf("qt: window shown (%s)\n", qVersion());
    std::fflush(stdout);
    return app.exec();
}
