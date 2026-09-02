// Sample for rulesets/cpp-default-guidelines.toml: every rule has a violation
// here, most with the compliant form beside it. Not real code.
#include <boost/algorithm/string.hpp>
#include <string>
#include <cstdarg>

typedef int handle_t;            // no-typedef
using handle2_t = int;           // fine

int g_mutable = 2;               // mutable-global, initialize-variables is not this (it has an init)
const int g_fixed = 1;           // fine
extern int g_ext;                // no-register-extern
double g_uninit;                 // initialize-variables

static_assert(sizeof(int) == 4, "");   // static-assert-in-header

struct Iface {                   // interface-without-data
    virtual ~Iface() = default;
    virtual void go() = 0;
    int state;                   // initialize-members
};

struct Widget {
public:
    Widget(int w);               // explicit-single-arg-ctor
    explicit Widget(double d);   // fine
    ~Widget();                   // virtual-destructor (no virtual, no override)
    int width() const;           // nodiscard-on-const-getters
    [[nodiscard]] int height() const;   // fine
    void resize(int w, int h) final;    // final-needs-rationale
protected:                       // no-protected
    int m_prot = 0;
private:
    int& m_ref;                  // no-reference-member, initialize-members
    double m_uninit;             // initialize-members
};

Widget::Widget(int w) { m_uninit = w; }   // initializer-list-not-ctor-body
Widget::Widget(double d) : m_uninit(d) {}  // fine

int* leak();                     // no-reference-return
const std::string& name(const std::string& s);   // no-reference-return, string-view-parameter
void view(std::string_view s);   // fine

void flag(bool on) {             // bool-parameter
    double local;                // initialize-variables
    for (int i = 0; i < 3; ++i) {          // raw-loop
        if (on) { if (i) { if (local) {} } } // two-levels-of-nesting
    }
    auto f = [=] { return 1; };  // lambda-default-capture
    auto g = [&local] { return local; };   // fine
}

void mem() {
    void* p = malloc(4);         // no-c-allocation
    free(p);
    int* q = reinterpret_cast<int*>(p);    // no-reinterpret-cast
    (void)q;
}

void var(int n, ...) {           // no-va-arg
    va_list ap;
    va_start(ap, n);
    va_end(ap);
}

void wide(int a, int b, int c, int d) {}   // three-arguments

int tall() {                     // function-size (43+ lines)
    int a = 0;
    a += 1;
    a += 2;
    a += 3;
    a += 4;
    a += 5;
    a += 6;
    a += 7;
    a += 8;
    a += 9;
    a += 10;
    a += 11;
    a += 12;
    a += 13;
    a += 14;
    a += 15;
    a += 16;
    a += 17;
    a += 18;
    a += 19;
    a += 20;
    a += 21;
    a += 22;
    a += 23;
    a += 24;
    a += 25;
    a += 26;
    a += 27;
    a += 28;
    a += 29;
    a += 30;
    a += 31;
    a += 32;
    a += 33;
    a += 34;
    a += 35;
    a += 36;
    a += 37;
    a += 38;
    a += 39;
    a += 40;
    return a;
}
