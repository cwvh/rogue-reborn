// mapkad.S: reads a .MAP with the schemas of mapk.h written out as code, an op a macro, as
// mapkd.c does, in assembly
#include "mapk.h"
#define ROOT lea r8, [rip+r_MAP]; mov ecx, wMAP; mov eax, 1; call rows
#include "mapka.inc"

// rax rows of ecx words, each read by the routine at r8, at the index left in eax; the row being
// read is kept
rows:
    push rbp
    push r10
    push r11
    push r9
    push rdx
    mov rbp, rax
    imul rax, rcx
    push rcx
    push r8
    call alloc
    pop r8
    pop rcx
    push rax
    lea r10, [rbx+rax*4]
    test rbp, rbp
    jz 2f
1:  push rbp
    push r10
    push rcx
    push r8
    call r8
    pop r8
    pop rcx
    pop r10
    pop rbp
    lea r10, [r10+rcx*4]
    dec rbp
    jnz 1b
2:  pop rax
    pop rdx
    pop r9
    pop r11
    pop r10
    pop rbp
    ret

// ?: a row of the dynamic object kind of the row's section id
kind:
    mov eax, [r11+4]
    lea rcx, [rip+kinds]
    xor edi, edi
1:  movzx r8d, byte ptr [rcx+rdi]
    cmp eax, r8d
    je 2f
    inc edi
    cmp edi, 7
    jb 1b
    jmp fail
2:  movzx eax, byte ptr [rcx+rdi+7]
    lea r8, [rip+kind_rows]
    mov r8, [r8+rdi*8]
    mov ecx, eax
    mov eax, 1
    call rows
    mov [r10], eax
    add r10, 4
    ret

// d: a count, then that many K rows, or one F row where it is 0
dyn:
    need 4
    mov eax, [rsi]
    add rsi, 4
    mov [r10], eax
    add r10, 4
    lea r8, [rip+r_K]
    mov ecx, wK
    test eax, eax
    jnz 1f
    lea r8, [rip+r_F]
    mov ecx, wF
    mov eax, 1
1:  call rows
    mov [r10], eax
    add r10, 4
    ret

// The widths, as the assembler's symbols, for the macros
.set W_M, wM
.set W_O, wO
.set W_D, wD
.set W_G, wG
.set W_P, wP
.set W_Y, wY
.set W_Q, wQ
.set W_V, wV
.set W_T, wT
.set W_L, wL
.set W_H, wH

// op  u f   b   s     z   S     R      n   k#    k%    $   @X       a      e      w        !      ?      d
//     xw k  xb  xs k  xz  xsec  xsecr  xn  xa k  xh k  xl  xrows X  xmode  xencl  xweather xzero  xkind  xdyn
.macro xw k
    .if \k == 1
    need 4
    mov eax, [rsi]
    mov [r10], eax
    add rsi, 4
    add r10, 4
    .else
    mov edx, \k
    call op_u
    .endif
.endm
.macro xb
    need 1
    movzx eax, byte ptr [rsi]
    inc rsi
    mov [r10], eax
    add r10, 4
.endm
.macro xs k
    mov edx, \k
    call op_s
.endm
.macro xz
    call op_z
.endm
.macro xsec
    call op_S
.endm
.macro xsecr
    call op_R
.endm
.macro xn
    need 4
    mov r9d, [rsi]
    add rsi, 4
    mov [r10], r9d
    add r10, 4
.endm
.macro xa k
    mov edx, \k
    call op_words
.endm
.macro xh k
    mov edx, \k
    call op_halves
.endm
.macro xl
    call op_strings
.endm
.macro xrows x
    lea r8, [rip+r_\x]
    mov ecx, W_\x
    mov eax, r9d
    call rows
    mov [r10], eax
    add r10, 4
.endm
.macro xmode
    call op_mode
.endm
.macro xencl
    call op_encl
.endm
.macro xweather
    call op_weather
.endm
.macro xzero
    call op_zero
.endm
.macro xkind
    call kind
.endm
.macro xdyn
    call dyn
.endm

// A routine reading a row, from r10
.macro rec name
r_\name:
    mov r11, r10
.endm

#define COMMON xw 12; xs 1; xw 1; xs 11

rec K;   xs 1; xw 11; ret
rec F;   xn; xl; xw 4; ret
rec H;   xs 1; xw 8; ret
rec 14;  COMMON; xdyn; ret
rec 15;  COMMON; xw 1; xn; xl; xw 3; xw 1; xs 3; xw 5; ret
rec 16;  COMMON; xw 1; xn; xl; xw 3; xn; xl; xs 3; xw 5; ret
rec 20;  xz; ret
rec 25;  xs 2; xw 6; xn; xl; ret
rec 31;  xn; xrows H; ret
rec 36;  ret

rec M;   xsec; xs 1; xw 2; xmode; xw 13; xb; ret
rec D;   xw 1; xn; xa 4; xh 3; xh 3; xn; xa 3; xa 2; xa 4; ret
rec G;   xz; xw 1; xn; xh 1; ret
rec O;   xsec; xsec; xn; xa 3; xn; xrows D; xn; xa 3; xn; xa 4; xn; xh 8; xn; xrows G; ret
rec P;   xsec; xn; xa 3; xw 2; ret
rec Y;   xsec; xs 1; xw 12; xkind; ret
rec V;   xs 1; xn; xa 18; xn; xa 1; xzero; ret
rec Q;   xsecr; xencl; xb; xweather; xn; xrows V; xn; xa 2; xn; xa 1; ret
rec T;   xs 1; xw 6; ret
rec L;   xw 2; xn; xl; ret
rec MAP; call op_begin; xw 1; xsec; xn; xrows M; xsec; xn; xrows O; xsec; xn; xrows P; xsec; xw 1
         xsec; xn; xrows Y; xsec; xn; xrows Q; xsec; xn; xrows T; xsec; xn; xrows L; call op_end; ret

.section .rdata,"dr"
// The dynamic object kinds' ids, then their widths
kinds: .byte 14, 15, 16, 20, 25, 31, 36, w14, w15, w16, w20, w25, w31, w36
.p2align 3
kind_rows: .quad r_14, r_15, r_16, r_20, r_25, r_31, r_36
