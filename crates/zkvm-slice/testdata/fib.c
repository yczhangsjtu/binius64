/* fib.c — M8-C T1 (optional): iterative fibonacci, result word to the flag address.
 * Same memory contract as bubble16.c (see its header). Result: fib(12) = 144.
 */
volatile unsigned int * const FLAG = (unsigned int *)0x400;

void _start(void) {
    unsigned int a = 0, b = 1;
    for (unsigned int i = 0; i < 12u; i++) {
        unsigned int t = a + b;
        a = b;
        b = t;
    }
    *FLAG = a; /* fib(12) = 144 */
    __asm__ volatile(".word 0x00000073");
}
