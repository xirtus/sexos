// SPDX-License-Identifier: GPL-3.0-only
// C++ in a SexOS program, on libc++ (scripts/build_libcxx.sh): the parts of the
// standard library a ported engine such as V8 leans on. Each check writes one line;
// the program reports how many passed.
#include <algorithm>
#include <atomic>
#include <cstdio>
#include <functional>
#include <map>
#include <memory>
#include <mutex>
#include <sstream>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>

namespace {

// A global with a constructor: runs only if the program runs .init_array.
struct Registry {
  std::vector<std::string> names;
  Registry() { names.push_back("constructed"); }
};
Registry registry;

thread_local int per_thread = 5;

struct Shape {
  virtual ~Shape() = default;
  virtual double area() const = 0;
};
struct Square : Shape {
  double side;
  explicit Square(double s) : side(s) {}
  double area() const override { return side * side; }
};

}  // namespace

extern "C" int cpp_demo(char* out, unsigned long size) {
  std::string report;
  int passed = 0, total = 0;
  auto check = [&](const char* name, bool ok) {
    ++total;
    if (ok) ++passed;
    report += name;
    report += ok ? " ok\n" : " FAILED\n";
  };

  check("static constructor", registry.names.size() == 1 && registry.names[0] == "constructed");

  std::vector<int> v;
  for (int i = 0; i < 1000; ++i) v.push_back((i * 7919) % 1000);
  std::sort(v.begin(), v.end());
  check("vector + sort", std::is_sorted(v.begin(), v.end()) && v.front() == 0 && v.back() == 999);

  std::unordered_map<std::string, int> words;
  for (const char* w : {"web", "gl", "web", "js", "web"}) ++words[w];
  check("unordered_map<string>", words["web"] == 3 && words.size() == 3);

  std::map<int, std::string> ordered{{3, "c"}, {1, "a"}, {2, "b"}};
  std::string joined;
  for (auto& [k, s] : ordered) joined += s;
  check("map", joined == "abc");

  std::unique_ptr<Shape> shape = std::make_unique<Square>(3.0);
  check("virtual call", shape->area() == 9.0);

  std::function<int(int)> twice = [](int x) { return x * 2; };
  check("std::function", twice(21) == 42);

  std::ostringstream stream;
  stream << "pi~" << 3.25 << " n=" << 42;
  check("ostringstream", stream.str() == "pi~3.25 n=42");

  char buf[32];
  std::snprintf(buf, sizeof buf, "%d-%s", 7, "x");
  check("snprintf", std::string(buf) == "7-x");

  // Threads: four std::threads add under a std::mutex; each has its own thread_local.
  std::mutex lock;
  long sum = 0;
  std::atomic<int> own_ok{0};
  std::vector<std::thread> threads;
  for (int t = 1; t <= 4; ++t) {
    threads.emplace_back([&, t] {
      per_thread += t;
      if (per_thread == 5 + t) own_ok.fetch_add(1);
      for (int i = 0; i < 1000; ++i) {
        std::lock_guard<std::mutex> held(lock);
        sum += t;
      }
    });
  }
  for (auto& th : threads) th.join();
  check("std::thread + mutex", sum == 10000);
  check("thread_local", own_ok.load() == 4 && per_thread == 5);

  std::snprintf(out, size, "%d/%d\n%s", passed, total, report.c_str());
  return passed == total ? 0 : 1;
}
