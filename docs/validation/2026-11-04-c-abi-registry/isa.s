	.intel_syntax noprefix
	.file	"isa.c"
	.text
	.globl	add_double                      # -- Begin function add_double
	.prefalign	4, .Lfunc_end0, nop
	.type	add_double,@function
add_double:                             # @add_double
# %bb.0:
	push	rbp
	mov	rbp, rsp
	vaddsd	xmm0, xmm0, xmm1
	pop	rbp
	ret
.Lfunc_end0:
	.size	add_double, .Lfunc_end0-add_double
                                        # -- End function
	.globl	add_extended                    # -- Begin function add_extended
	.prefalign	4, .Lfunc_end1, nop
	.type	add_extended,@function
add_extended:                           # @add_extended
# %bb.0:
	push	rbp
	mov	rbp, rsp
	fld	tbyte ptr [rbp + 32]
	fld	tbyte ptr [rbp + 16]
	faddp	st(1), st
	pop	rbp
	ret
.Lfunc_end1:
	.size	add_extended, .Lfunc_end1-add_extended
                                        # -- End function
	.globl	fused_multiply_add              # -- Begin function fused_multiply_add
	.prefalign	4, .Lfunc_end2, nop
	.type	fused_multiply_add,@function
fused_multiply_add:                     # @fused_multiply_add
# %bb.0:
	push	rbp
	mov	rbp, rsp
	vfmadd213sd	xmm0, xmm1, xmm2        # xmm0 = (xmm1 * xmm0) + xmm2
	pop	rbp
	ret
.Lfunc_end2:
	.size	fused_multiply_add, .Lfunc_end2-fused_multiply_add
                                        # -- End function
	.globl	add_vector                      # -- Begin function add_vector
	.prefalign	4, .Lfunc_end3, nop
	.type	add_vector,@function
add_vector:                             # @add_vector
# %bb.0:
	push	rbp
	mov	rbp, rsp
	vpaddd	ymm0, ymm1, ymm0
	pop	rbp
	ret
.Lfunc_end3:
	.size	add_vector, .Lfunc_end3-add_vector
                                        # -- End function
	.globl	leading_zeros                   # -- Begin function leading_zeros
	.prefalign	4, .Lfunc_end4, nop
	.type	leading_zeros,@function
leading_zeros:                          # @leading_zeros
# %bb.0:
	push	rbp
	mov	rbp, rsp
	lzcnt	rax, rdi
	pop	rbp
	ret
.Lfunc_end4:
	.size	leading_zeros, .Lfunc_end4-leading_zeros
                                        # -- End function
	.globl	low_bits                        # -- Begin function low_bits
	.prefalign	4, .Lfunc_end5, nop
	.type	low_bits,@function
low_bits:                               # @low_bits
# %bb.0:
	push	rbp
	mov	rbp, rsp
	mov	eax, esi
	bzhi	rax, rdi, rax
	pop	rbp
	ret
.Lfunc_end5:
	.size	low_bits, .Lfunc_end5-low_bits
                                        # -- End function
	.ident	"clang version 23.1.4"
	.section	".note.GNU-stack","",@progbits
	.addrsig
