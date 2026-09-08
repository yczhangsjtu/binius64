/* bubble16.c — M8-C T1: real-compiled bubblesort for the binary-field zkVM.
 *
 * Memory contract (byte addressing; RAM = 2^16 words = 256KB, addr mask 0x3ffff):
 *   text:  0x00000000 (linked via -Ttext=0; rv32im, no compressed instrs)
 *   data:  .data section holds the 16-word input array (ELF LOAD segment;
 *          loader places it at its p_paddr/p_vma word address)
 *   stack: grows down from 0x3fffc (sp initialised by _start)
 *   flag:  word at byte address 0x400 := 0xC0DE600D on completion (completion marker)
 * Output: the sorted array IN PLACE (the verifier reads final memory at the
 *         array's word addresses) + flag word.
 * Halt:   `ecall` (0x00000073).
 */
volatile unsigned int * const FLAG = (unsigned int *)0x400;

unsigned int arr[16] = {
    0x80000000u, /* INT_MIN  (boundary)        */
    0x00000005u,
    0x00000005u, /* duplicate                  */
    0xffffffffu, /* UINT_MAX (boundary)        */
    0x00000000u, /* zero                       */
    0x7fffffffu,
    0x00000003u,
    0x00000003u, /* duplicate                  */
    0xdeadbeefu,
    0x00000001u,
    0xcafebabeu,
    0x00000002u,
    0x12345678u,
    0x00000004u,
    0x00000004u, /* duplicate                  */
    0x00000042u,
};

void _start(void) {
    for (unsigned int i = 0; i < 15u; i++) {
        for (unsigned int j = 0; j < 15u - i; j++) {
            if (arr[j] > arr[j + 1u]) {
                unsigned int t = arr[j];
                arr[j] = arr[j + 1u];
                arr[j + 1u] = t;
            }
        }
    }
    *FLAG = 0xC0DE600Du;
    __asm__ volatile(".word 0x00000073"); /* ecall = halt */
}
