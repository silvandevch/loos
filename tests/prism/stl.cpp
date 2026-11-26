// The C++ standard library (LLVM libc++, /usr/lib/libc++.1.dylib) under Prism: strings,
// containers, algorithms, streams, smart pointers, std::function, exceptions from the library,
// threads and std::filesystem. Built against the libc++ headers Prism's libc++ was built from.
#include <algorithm>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <functional>
#include <map>
#include <memory>
#include <mutex>
#include <numeric>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>
#include <iostream>

static int dtors;
struct Tracked { int v; explicit Tracked(int x) : v(x) {} ~Tracked() { dtors++; } };

int main() {
    // Strings and string streams.
    std::string s = "hello";
    s += ", prism";
    s.insert(0, ">");
    std::ostringstream os;
    os << s << " " << 42 << " " << 2.5 << " " << std::hex << 255;
    std::istringstream is("7 11 13");
    int a, b, c;
    is >> a >> b >> c;
    std::printf("str: %s len=%zu find=%zu sum=%d up=%s num=%d\n", os.str().c_str(), s.size(), s.find("prism"),
                a + b + c, [&] { std::string u = s; std::transform(u.begin(), u.end(), u.begin(), ::toupper); return u; }().c_str(),
                std::stoi("-123") + std::stoi(std::to_string(200)));

    // Containers and algorithms.
    std::vector<int> v(10);
    std::iota(v.begin(), v.end(), 1);
    std::reverse(v.begin(), v.end());
    std::sort(v.begin(), v.end());
    std::map<std::string, int> m{{"b", 2}, {"a", 1}, {"c", 3}};
    std::unordered_map<int, std::string> um;
    for (int i = 0; i < 1000; i++) um[i] = std::to_string(i * i);
    std::string keys;
    for (auto &[k, val] : m) keys += k;
    std::printf("cont: sum=%d first=%d keys=%s um=%zu/%s\n", std::accumulate(v.begin(), v.end(), 0), v.front(),
                keys.c_str(), um.size(), um[31].c_str());

    // Smart pointers, std::function.
    {
        auto u = std::make_unique<Tracked>(5);
        auto sp = std::make_shared<Tracked>(6);
        auto sp2 = sp;
        std::function<int(int)> f = [p = sp](int x) { return x * p->v; };
        std::printf("ptr: u=%d use=%ld f=%d\n", u->v, sp.use_count(), f(7));
    }

    // Exceptions thrown by the library itself.
    int caught = 0;
    try { std::vector<int>().at(3); } catch (const std::out_of_range &) { caught++; }
    try { std::stoi("nope"); } catch (const std::invalid_argument &e) { caught += 10; }
    try { throw std::runtime_error("boom"); } catch (const std::exception &e) { if (std::string(e.what()) == "boom") caught += 100; }
    std::printf("exc: caught=%d dtors=%d\n", caught, dtors);

    // Threads.
    std::mutex mu;
    long total = 0;
    std::vector<std::thread> ts;
    for (int t = 0; t < 4; t++)
        ts.emplace_back([&, t] { for (int i = 0; i < 1000; i++) { std::lock_guard<std::mutex> g(mu); total += t; } });
    for (auto &t : ts) t.join();
    std::printf("thr: total=%ld\n", total);

    // Files and std::filesystem.
    namespace fs = std::filesystem;
    fs::path dir = "/tmp/stl_test";
    fs::create_directories(dir / "sub");
    { std::ofstream out(dir / "sub" / "f.txt"); out << "line1\nline2\n"; }
    std::ifstream in(dir / "sub" / "f.txt");
    std::string line, last;
    int lines = 0;
    while (std::getline(in, line)) { lines++; last = line; }
    std::printf("fs: lines=%d last=%s size=%ju exists=%d ext=%s\n", lines, last.c_str(),
                (uintmax_t)fs::file_size(dir / "sub" / "f.txt"), (int)fs::exists(dir / "sub"),
                fs::path("a/b.tar.gz").extension().c_str());
    std::cout << "cout: " << fs::remove_all(dir) << " removed" << std::endl;
    return 0;
}
