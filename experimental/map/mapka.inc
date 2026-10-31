// mapka.inc: what the assembly .MAP readers mapkai.S and mapkad.S share. Each defines ROOT, the
// instructions that read the map's own row, then includes this.
//
// They build the same map as mapki.c and mapkd.c, laid out as mapk.h says. Through the whole
// parse these registers hold:
//   rsi  the file, read from here up      r12  its end
//   rbx  the map's words                  r13  the words used    r14  the words there are
//   r15  the stack pointer to fail back to
// and for the row being read:
//   r10  where its next word goes         r11  where it starts   r9d  the count `n` read
// A failure, from anywhere, puts the stack back as mapk_parse found it and returns 0.
//
// The ops are subroutines taking a count, where they have one, in edx. Besides rsi, r10, r13 and
// what they say, each may change rax, rcx, rdi, r8, xmm0 and xmm1.

.intel_syntax noprefix
.text

// Fail unless n bytes of the file are left, using t
.macro need n, t=rdi
    mov \t, r12
    sub \t, rsi
    cmp \t, \n
    jb fail
.endm

// --- The calls from C ---------------------------------------------------------------------

// uint32_t mapk_parse(const void *d, ptrdiff_t n, uint32_t *w, ptrdiff_t m)
.globl mapk_parse
mapk_parse:
    push rbx
    push rbp
    push rsi
    push rdi
    push r12
    push r13
    push r14
    push r15
    mov rsi, rcx
    lea r12, [rcx+rdx]
    mov rbx, r8
    xor r13d, r13d
    // The words there are, from 0 to the most a word's index can reach
    mov r14, r9
    test r14, r14
    cmovs r14, r13
    mov eax, 0xffffffff
    cmp r14, rax
    cmova r14, rax
    mov r15, rsp
    ROOT
    mov eax, r13d
    jmp 1f
fail:
    mov rsp, r15
    xor eax, eax
1:  pop r15
    pop r14
    pop r13
    pop r12
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    ret

// uint32_t mapk_read(const char *path, uint32_t *w, ptrdiff_t m): the file goes at the top of w
.globl mapk_read
mapk_read:
    push rbx
    push rsi
    push rdi
    push r12
    sub rsp, 40
    mov rsi, rdx
    mov rdi, r8
    lea rdx, [rip+mode_rb]
    call fopen
    test rax, rax
    jz 3f
    mov rbx, rax
    mov rcx, rbx
    xor edx, edx
    mov r8d, 2                  // SEEK_END
    call fseek
    test eax, eax
    jnz 2f
    mov rcx, rbx
    call ftell
    movsxd r12, eax
    test r12, r12
    js 2f
    lea rax, [rdi*4]
    cmp r12, rax
    ja 2f
    mov rcx, rbx
    xor edx, edx
    xor r8d, r8d                // SEEK_SET
    call fseek
    test eax, eax
    jnz 2f
    lea rdi, [rsi+rdi*4]
    sub rdi, r12
    mov rcx, rdi
    mov edx, 1
    mov r8, r12
    mov r9, rbx
    call fread
    cmp rax, r12
    jne 2f
    mov rcx, rbx
    call fclose
    mov rcx, rdi
    mov rdx, r12
    mov r8, rsi
    mov r9, rdi
    sub r9, rsi
    shr r9, 2
    call mapk_parse
    jmp 4f
2:  mov rcx, rbx
    call fclose
3:  xor eax, eax
4:  add rsp, 40
    pop r12
    pop rdi
    pop rsi
    pop rbx
    ret

// --- Primitives ---------------------------------------------------------------------------

// rax words of the map, at the index left in rax
alloc:
    mov rcx, r14
    sub rcx, r13
    cmp rax, rcx
    ja fail
    mov rcx, r13
    add r13, rax
    mov rax, rcx
    ret

// rcx bytes from rsi to rdi, moving rsi past them, as rep movsb does but quicker for the short
// strings and arrays most of a map is: overlapping loads and stores that stay within the bytes.
// Uses xmm0 and xmm1 too.
copy:
    cmp rcx, 16
    ja 3f
    cmp rcx, 8
    jb 1f
    movq xmm0, [rsi]            // 8 to 16
    movq xmm1, [rsi+rcx-8]
    movq [rdi], xmm0
    movq [rdi+rcx-8], xmm1
    add rsi, rcx
    ret
1:  cmp rcx, 4
    jb 2f
    movd xmm0, [rsi]            // 4 to 7
    movd xmm1, [rsi+rcx-4]
    movd [rdi], xmm0
    movd [rdi+rcx-4], xmm1
    add rsi, rcx
    ret
2:  rep movsb                   // up to 3, and from 256
    ret
3:  cmp rcx, 256
    jae 2b
    movdqu xmm1, [rsi+rcx-16]   // 17 to 255: 16 at a time, then the last 16
    lea rax, [rdi+rcx-16]
4:  movdqu xmm0, [rsi]
    movdqu [rdi], xmm0
    add rsi, 16
    add rdi, 16
    sub rcx, 16
    cmp rcx, 16
    ja 4b
    movdqu [rax], xmm1
    add rsi, rcx
    ret

// A string: its length, counting a NUL, then its bytes, kept without the last byte and with a
// NUL after them. Leaves its index in eax and the length kept in r8.
string:
    need 4
    mov r8d, [rsi]
    add rsi, 4
    test r8d, r8d
    jz fail
    need r8
    lea rax, [r8+3]
    shr rax, 2
    mov rdi, rax
    call alloc
    lea rcx, [rax+rdi-1]
    mov dword ptr [rbx+rcx*4], 0
    lea rdi, [rbx+rax*4]
    dec r8
    mov rcx, r8
    push rax
    call copy
    pop rax
    inc rsi
    ret

// Fail unless the bytes at rdi, up to a NUL, are UTF-8 as Rust takes it; changes rdx too
utf8:
    movzx eax, byte ptr [rdi]
    inc rdi
    cmp eax, 0x80
    jb 5f
    cmp eax, 0xc2
    jb fail
    cmp eax, 0xe0
    jae 2f
    and eax, 0x1f
    mov ecx, 1
    mov edx, 0x80
    jmp 4f
2:  cmp eax, 0xf0
    jae 3f
    and eax, 0x0f
    mov ecx, 2
    mov edx, 0x800
    jmp 4f
3:  cmp eax, 0xf4
    ja fail
    and eax, 0x07
    mov ecx, 3
    mov edx, 0x10000
4:  movzx r8d, byte ptr [rdi]   // a byte that goes on: 10xxxxxx
    inc rdi
    sub r8d, 0x80
    cmp r8d, 0x3f
    ja fail
    shl eax, 6
    or eax, r8d
    dec ecx
    jnz 4b
    cmp eax, edx                // the shortest form
    jb fail
    cmp eax, 0x10ffff
    ja fail
    mov ecx, eax
    and ecx, 0xfffff800         // no surrogates
    cmp ecx, 0xd800
    je fail
    jmp utf8
5:  test eax, eax
    jnz utf8
    ret

// The rest of a section header, after its size: the id, then the name, or "Version", a number
// and the name. Writes the id, the number or 0 and the name.
sect:
    need 4
    mov eax, [rsi]
    add rsi, 4
    mov [r10], eax
    mov dword ptr [r10+4], 0
    call string
    mov [r10+8], eax
    cmp r8d, 7
    jne 1f
    mov rcx, 0x006e6f6973726556 // "Version" and its NUL
    cmp [rbx+rax*4], rcx
    jne 1f
    need 4
    mov eax, [rsi]
    add rsi, 4
    mov [r10+4], eax
    call string
    mov [r10+8], eax
1:  add r10, 12
    ret

// Fail unless the next string is the ecx bytes at rdi
magic:
    need 4, rax
    mov eax, [rsi]
    add rsi, 4
    dec eax
    cmp eax, ecx
    jne fail
    lea r8, [rcx+1]
    need r8, rax
    repe cmpsb
    jne fail
    inc rsi
    ret

// --- Ops ----------------------------------------------------------------------------------

// u f: edx words, at least 1
op_u:
    test edx, edx
    jnz 1f
    inc edx
1:  lea rcx, [rdx*4]
    need rcx
2:  mov eax, [rsi]
    mov [r10], eax
    add rsi, 4
    add r10, 4
    dec edx
    jnz 2b
    ret

// b: a byte
op_b:
    need 1
    movzx eax, byte ptr [rsi]
    inc rsi
    mov [r10], eax
    add r10, 4
    ret

// s: edx strings, at least 1
op_s:
    test edx, edx
    jnz 1f
    inc edx
1:  call string
    mov [r10], eax
    add r10, 4
    dec edx
    jnz 1b
    ret

// z: a string that is UTF-8
op_z:
    call string
    mov [r10], eax
    add r10, 4
    lea rdi, [rbx+rax*4]
    jmp utf8

// S: a section
op_S:
    need 4
    mov eax, [rsi]
    add rsi, 4
    mov [r10], eax
    add r10, 4
    jmp sect

// R: a section with no size
op_R:
    mov dword ptr [r10], 0
    add r10, 4
    jmp sect

// n: the count
op_n:
    need 4
    mov r9d, [rsi]
    add rsi, 4
    mov [r10], r9d
    add r10, 4
    ret

// #: n rows of edx words
op_words:
    mov eax, r9d
    imul rax, rdx
    mov r8, rax
    call alloc
    mov [r10], eax
    add r10, 4
    lea rcx, [r8*4]
    need rcx
    lea rdi, [rbx+rax*4]
    jmp copy

// %: n rows of edx u16s, with the last word they reach zeroed
op_halves:
    mov eax, r9d
    imul rax, rdx
    add rax, rax
    mov r8, rax
    add rax, 3
    shr rax, 2
    mov rdx, rax
    call alloc
    mov [r10], eax
    add r10, 4
    test rdx, rdx
    jz 1f
    lea rcx, [rax+rdx-1]
    mov dword ptr [rbx+rcx*4], 0
1:  mov rcx, r8
    need rcx
    lea rdi, [rbx+rax*4]
    jmp copy

// $: n strings; changes rdx too
op_strings:
    mov eax, r9d
    call alloc
    mov [r10], eax
    add r10, 4
    lea rdx, [rbx+rax*4]
    push r9
    test r9d, r9d
    jz 2f
1:  call string
    mov [rdx], eax
    add rdx, 4
    dec r9d
    jnz 1b
2:  pop r9
    ret

// a: the texture address mode, 0 1 or 3
op_mode:
    need 4
    mov eax, [rsi]
    add rsi, 4
    mov [r10], eax
    add r10, 4
    cmp eax, 3
    ja fail
    cmp eax, 2
    je fail
    ret

// e: whether the room is enclosed, then where it is not whether it is small, or -1
op_encl:
    need 1
    movzx eax, byte ptr [rsi]
    inc rsi
    mov [r10], eax
    test eax, eax
    jz 1f
    cmp eax, 1
    ja fail
    mov dword ptr [r10+4], -1
    add r10, 8
    ret
1:  need 1
    movzx eax, byte ptr [rsi]
    inc rsi
    cmp eax, 1
    ja fail
    mov [r10+4], eax
    add r10, 8
    ret

// w: where the row's version is above 1 whether it has weather bounds, or -1, then the bounds
// where it has them, or 0s
op_weather:
    mov eax, -1
    cmp dword ptr [r11+8], 1
    jbe 1f
    need 1
    movzx eax, byte ptr [rsi]
    inc rsi
1:  mov [r10], eax
    xor ecx, ecx
    mov [r10+4], rcx
    mov [r10+12], rcx
    mov [r10+20], rcx
    lea ecx, [rax+1]            // 0 and -1 have none
    cmp ecx, 1
    jbe 2f
    need 24
    mov rcx, [rsi]
    mov [r10+4], rcx
    mov rcx, [rsi+8]
    mov [r10+12], rcx
    mov rcx, [rsi+16]
    mov [r10+20], rcx
    add rsi, 24
2:  add r10, 28
    ret

// !: a byte that must be 0
op_zero:
    need 1
    movzx eax, byte ptr [rsi]
    inc rsi
    mov [r10], eax
    add r10, 4
    test eax, eax
    jnz fail
    ret

// < >: the magic at the start and the end
op_begin:
    lea rdi, [rip+magic_begin]
    mov ecx, 12
    jmp magic
op_end:
    lea rdi, [rip+magic_end]
    mov ecx, 6
    jmp magic

.section .rdata,"dr"
mode_rb: .asciz "rb"
magic_begin: .ascii "BeginMapv2.1"
magic_end: .ascii "EndMap"
.text
