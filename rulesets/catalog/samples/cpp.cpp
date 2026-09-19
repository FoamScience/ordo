// Exercises every C/C++ rule in rulesets/catalog/c-cpp.toml.
#include <csetjmp>
#include <cstdio>
#include <cstdlib>
#include <cstring>

#define SQUARE(x) ((x) * (x))   // function-macro

struct Oops {};

volatile int g_flag = 0;        // volatile

void copy_in(char *dst, const char *src) {
    strcpy(dst, src);           // unsafe-str-fn
}

int classify(void *p) {
    int n = (int)(long)p;       // c-style-cast
    if (n < 0) {
        goto done;              // goto
    }
    n = 1;
done:
    return n;
}

void memory() {
    int *a = new int(3);        // raw-new-delete
    delete a;                   // raw-new-delete
    void *b = malloc(16);       // manual-memory
    memset(b, 0, 16);           // mem-family
    free(b);                    // manual-memory
    void *c = alloca(8);        // alloca
    (void)c;
}

void runtime_calls() {
    system("ls");               // shell-exec
    int r = rand();             // non-reentrant
    (void)r;
    std::jmp_buf env;
    setjmp(env);                // setjmp-longjmp
}

void capture(int n) {
    auto f = [&] { return n; }; // lambda-ref-capture
    f();
}

void handler() {
    try {
        throw Oops{};
    } catch (Oops e) {          // catch-by-value
        (void)e;
    }
}
