// mapkai.S: reads a .MAP by interpreting the schemas of mapk.h, as mapki.c does, in assembly
#include "mapk.h"
#define ROOT lea r8, [rip+k_MAP]; mov eax, 1; call rows
#include "mapka.inc"

// rax rows of the schema at r8, at the index left in eax; the row being read is kept
rows:
    push rbp
    push r10
    push r11
    push r9
    push rdx
    mov ecx, [r8-4]             // the width, kept before the schema
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
    mov rbp, r8
    call row
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

// A row of the schema at rbp: each op, after the number in front of it, which goes in edx
row:
    mov r11, r10
1:  movzx eax, byte ptr [rbp]
    inc rbp
    test eax, eax
    jz 4f
    xor edx, edx
2:  lea ecx, [rax-0x30]
    cmp ecx, 9
    ja 3f
    lea edx, [rdx+rdx*4]
    lea edx, [rcx+rdx*2]
    movzx eax, byte ptr [rbp]
    inc rbp
    jmp 2b
3:  lea rcx, [rip+ops]
    movsx rax, word ptr [rcx+rax*2-2*0x21]
    lea rcx, [rip+fail]
    add rax, rcx
    call rax
    jmp 1b
4:  ret

// @X: n rows of the schema named by the letter after the @
op_at:
    movzx eax, byte ptr [rbp]
    inc rbp
    lea rcx, [rip+letters]
    xor edi, edi
1:  cmp al, [rcx+rdi]
    je 2f
    inc edi
    jmp 1b
2:  lea rcx, [rip+schemas]
    movsxd r8, dword ptr [rcx+rdi*4]
    add r8, rcx
    mov eax, r9d
    call rows
    mov [r10], eax
    add r10, 4
    ret

// ?: a row of the dynamic object kind of the row's section id
op_kind:
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
2:  lea rcx, [rip+kind_schemas]
    movsxd r8, dword ptr [rcx+rdi*4]
    add r8, rcx
    mov eax, 1
    call rows
    mov [r10], eax
    add r10, 4
    ret

// d: a count, then that many K rows, or one F row where it is 0
op_dyn:
    need 4
    mov eax, [rsi]
    add rsi, 4
    mov [r10], eax
    add r10, 4
    lea r8, [rip+k_K]
    test eax, eax
    jnz 1f
    lea r8, [rip+k_F]
    mov eax, 1
1:  call rows
    mov [r10], eax
    add r10, 4
    ret

// Each op's offset from fail, by its character; any other is fail's
.macro op c, at
    .if ops + 2*(\c - 0x21) - . > 0
    .skip ops + 2*(\c - 0x21) - ., 0
    .endif
    .short \at - fail
.endm
ops:
    op 0x21, op_zero            // !
    op 0x23, op_words           // #
    op 0x24, op_strings         // $
    op 0x25, op_halves          // %
    op 0x3c, op_begin           // <
    op 0x3e, op_end             // >
    op 0x3f, op_kind            // ?
    op 0x40, op_at              // @
    op 0x52, op_R               // R
    op 0x53, op_S               // S
    op 0x61, op_mode            // a
    op 0x62, op_b               // b
    op 0x64, op_dyn             // d
    op 0x65, op_encl            // e
    op 0x66, op_u               // f
    op 0x6e, op_n               // n
    op 0x73, op_s               // s
    op 0x75, op_u               // u
    op 0x77, op_weather         // w
    op 0x7a, op_z               // z

.section .rdata,"dr"
letters: .ascii "MODGPYQVTLKFH"
.p2align 2
schemas:
    .long k_M-schemas, k_O-schemas, k_D-schemas, k_G-schemas, k_P-schemas, k_Y-schemas, k_Q-schemas
    .long k_V-schemas, k_T-schemas, k_L-schemas, k_K-schemas, k_F-schemas, k_H-schemas
kinds: .byte 14, 15, 16, 20, 25, 31, 36
.p2align 2
kind_schemas:
    .long k_14-kind_schemas, k_15-kind_schemas, k_16-kind_schemas, k_20-kind_schemas
    .long k_25-kind_schemas, k_31-kind_schemas, k_36-kind_schemas

// Each schema, after the width of its rows
.p2align 2
    .long wMAP
k_MAP: .asciz K_MAP
.p2align 2
    .long wM
k_M: .asciz K_M
.p2align 2
    .long wO
k_O: .asciz K_O
.p2align 2
    .long wD
k_D: .asciz K_D
.p2align 2
    .long wG
k_G: .asciz K_G
.p2align 2
    .long wP
k_P: .asciz K_P
.p2align 2
    .long wY
k_Y: .asciz K_Y
.p2align 2
    .long wQ
k_Q: .asciz K_Q
.p2align 2
    .long wV
k_V: .asciz K_V
.p2align 2
    .long wT
k_T: .asciz K_T
.p2align 2
    .long wL
k_L: .asciz K_L
.p2align 2
    .long wK
k_K: .asciz K_K
.p2align 2
    .long wF
k_F: .asciz K_F
.p2align 2
    .long wH
k_H: .asciz K_H
.p2align 2
    .long w14
k_14: .asciz K_14
.p2align 2
    .long w15
k_15: .asciz K_15
.p2align 2
    .long w16
k_16: .asciz K_16
.p2align 2
    .long w20
k_20: .asciz K_20
.p2align 2
    .long w25
k_25: .asciz K_25
.p2align 2
    .long w31
k_31: .asciz K_31
.p2align 2
    .long w36
k_36: .asciz K_36
