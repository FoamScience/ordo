/*
 * One violation of every rule in c-power-of-ten.toml, the compliant form
 * beside it where that's cheap. Check with:
 *   python3 scripts/ruleset-check.py rulesets/c-power-of-ten.toml rulesets/samples/c-power-of-ten.c
 */
#include <setjmp.h>
#include <stdlib.h>

/* [6] file-scope declaration */
static int g_counter = 0;

static jmp_buf g_recovery;

/* [8] function-like macro — not type-checked */
#define SQUARE(x) ((x) * (x))

/* [8] nested conditional compilation */
#ifdef FEATURE_A
#ifdef FEATURE_B
static int g_both = 1;
#endif
#endif

/* [1] setjmp/longjmp are unstructured control flow */
void jump_around(void) {
    if (setjmp(g_recovery) == 0) {
        longjmp(g_recovery, 1);
    }
}

/* [1] direct recursion */
int factorial(int n) {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}

/* [1] compliant: no recursion */
int factorial_iterative(int n) {
    int result = 1;
    for (int i = 2; i <= n; i++) {
        result *= i;
    }
    return result;
}

/* [2] unbounded loops */
void spin_for(void) {
    for (;;) {
        g_counter++;
        if (g_counter > 1000000) {
            break;
        }
    }
}

void spin_while(void) {
    while (1) {
        g_counter--;
        if (g_counter <= 0) {
            break;
        }
    }
}

/* [2] compliant: fixed bound */
void bounded_loop(void) {
    for (int i = 0; i < 100; i++) {
        g_counter += i;
    }
}

/* [3] dynamic memory after init */
int *allocate_buffer(int n) {
    int *buf = malloc((size_t)n * sizeof(int));
    return buf;
}

void release_buffer(int *buf) {
    free(buf);
}

/* [9] more than one level of pointer dereferencing */
void indirect(int **pp) {
    **pp = 0;
}

/* [9] compliant: one level */
void direct(int *p) {
    *p = 0;
}

/* [9] function pointer */
void register_callback(int (*callback)(int)) {
    (void)callback;
}

/* [4] over 60 lines */
int long_function(int x) {
    int a0 = x;
    int a1 = a0 + 1;
    int a2 = a1 + 1;
    int a3 = a2 + 1;
    int a4 = a3 + 1;
    int a5 = a4 + 1;
    int a6 = a5 + 1;
    int a7 = a6 + 1;
    int a8 = a7 + 1;
    int a9 = a8 + 1;
    int a10 = a9 + 1;
    int a11 = a10 + 1;
    int a12 = a11 + 1;
    int a13 = a12 + 1;
    int a14 = a13 + 1;
    int a15 = a14 + 1;
    int a16 = a15 + 1;
    int a17 = a16 + 1;
    int a18 = a17 + 1;
    int a19 = a18 + 1;
    int a20 = a19 + 1;
    int a21 = a20 + 1;
    int a22 = a21 + 1;
    int a23 = a22 + 1;
    int a24 = a23 + 1;
    int a25 = a24 + 1;
    int a26 = a25 + 1;
    int a27 = a26 + 1;
    int a28 = a27 + 1;
    int a29 = a28 + 1;
    int a30 = a29 + 1;
    int a31 = a30 + 1;
    int a32 = a31 + 1;
    int a33 = a32 + 1;
    int a34 = a33 + 1;
    int a35 = a34 + 1;
    int a36 = a35 + 1;
    int a37 = a36 + 1;
    int a38 = a37 + 1;
    int a39 = a38 + 1;
    int a40 = a39 + 1;
    int a41 = a40 + 1;
    int a42 = a41 + 1;
    int a43 = a42 + 1;
    int a44 = a43 + 1;
    int a45 = a44 + 1;
    int a46 = a45 + 1;
    int a47 = a46 + 1;
    int a48 = a47 + 1;
    int a49 = a48 + 1;
    int a50 = a49 + 1;
    int a51 = a50 + 1;
    int a52 = a51 + 1;
    int a53 = a52 + 1;
    int a54 = a53 + 1;
    int a55 = a54 + 1;
    int a56 = a55 + 1;
    int a57 = a56 + 1;
    int a58 = a57 + 1;
    return a58;
}

int main(void) {
    jump_around();
    (void)factorial(5);
    (void)factorial_iterative(5);
    spin_for();
    spin_while();
    bounded_loop();
    int *buf = allocate_buffer(4);
    release_buffer(buf);
    int v = 0;
    int *vp = &v;
    int **vpp = &vp;
    indirect(vpp);
    direct(vp);
    register_callback(NULL);
    (void)long_function(0);
    (void)SQUARE(2);
    return 0;
}
