/* Interrupt entry stubs: push (error code), vector, save GPRs, call the Rust dispatcher, iretq. */
.text
.extern loos_trap_dispatch
.macro SAVE
    push %rax; push %rbx; push %rcx; push %rdx; push %rsi; push %rdi; push %rbp
    push %r8; push %r9; push %r10; push %r11; push %r12; push %r13; push %r14; push %r15
.endm
.macro RESTORE
    pop %r15; pop %r14; pop %r13; pop %r12; pop %r11; pop %r10; pop %r9; pop %r8
    pop %rbp; pop %rdi; pop %rsi; pop %rdx; pop %rcx; pop %rbx; pop %rax
.endm
loos_isr_common:
    SAVE
    testb $3, 8*17+8(%rsp)
    jz 1f
    swapgs
1:  cld
    mov %rsp, %rdi
    call loos_trap_dispatch
    testb $3, 8*17+8(%rsp)
    jz 2f
    swapgs
2:  RESTORE
    add $16, %rsp                 /* vector + error code */
    iretq

.global loos_isr_0
loos_isr_0:
    pushq $0
    pushq $0
    jmp loos_isr_common

.global loos_isr_1
loos_isr_1:
    pushq $0
    pushq $1
    jmp loos_isr_common

.global loos_isr_2
loos_isr_2:
    pushq $0
    pushq $2
    jmp loos_isr_common

.global loos_isr_3
loos_isr_3:
    pushq $0
    pushq $3
    jmp loos_isr_common

.global loos_isr_4
loos_isr_4:
    pushq $0
    pushq $4
    jmp loos_isr_common

.global loos_isr_5
loos_isr_5:
    pushq $0
    pushq $5
    jmp loos_isr_common

.global loos_isr_6
loos_isr_6:
    pushq $0
    pushq $6
    jmp loos_isr_common

.global loos_isr_7
loos_isr_7:
    pushq $0
    pushq $7
    jmp loos_isr_common

.global loos_isr_8
loos_isr_8:
    pushq $8
    jmp loos_isr_common

.global loos_isr_9
loos_isr_9:
    pushq $0
    pushq $9
    jmp loos_isr_common

.global loos_isr_10
loos_isr_10:
    pushq $10
    jmp loos_isr_common

.global loos_isr_11
loos_isr_11:
    pushq $11
    jmp loos_isr_common

.global loos_isr_12
loos_isr_12:
    pushq $12
    jmp loos_isr_common

.global loos_isr_13
loos_isr_13:
    pushq $13
    jmp loos_isr_common

.global loos_isr_14
loos_isr_14:
    pushq $14
    jmp loos_isr_common

.global loos_isr_15
loos_isr_15:
    pushq $0
    pushq $15
    jmp loos_isr_common

.global loos_isr_16
loos_isr_16:
    pushq $0
    pushq $16
    jmp loos_isr_common

.global loos_isr_17
loos_isr_17:
    pushq $17
    jmp loos_isr_common

.global loos_isr_18
loos_isr_18:
    pushq $0
    pushq $18
    jmp loos_isr_common

.global loos_isr_19
loos_isr_19:
    pushq $0
    pushq $19
    jmp loos_isr_common

.global loos_isr_20
loos_isr_20:
    pushq $0
    pushq $20
    jmp loos_isr_common

.global loos_isr_21
loos_isr_21:
    pushq $21
    jmp loos_isr_common

.global loos_isr_22
loos_isr_22:
    pushq $0
    pushq $22
    jmp loos_isr_common

.global loos_isr_23
loos_isr_23:
    pushq $0
    pushq $23
    jmp loos_isr_common

.global loos_isr_24
loos_isr_24:
    pushq $0
    pushq $24
    jmp loos_isr_common

.global loos_isr_25
loos_isr_25:
    pushq $0
    pushq $25
    jmp loos_isr_common

.global loos_isr_26
loos_isr_26:
    pushq $0
    pushq $26
    jmp loos_isr_common

.global loos_isr_27
loos_isr_27:
    pushq $0
    pushq $27
    jmp loos_isr_common

.global loos_isr_28
loos_isr_28:
    pushq $0
    pushq $28
    jmp loos_isr_common

.global loos_isr_29
loos_isr_29:
    pushq $29
    jmp loos_isr_common

.global loos_isr_30
loos_isr_30:
    pushq $30
    jmp loos_isr_common

.global loos_isr_31
loos_isr_31:
    pushq $0
    pushq $31
    jmp loos_isr_common

.global loos_isr_32
loos_isr_32:
    pushq $0
    pushq $32
    jmp loos_isr_common

.global loos_isr_33
loos_isr_33:
    pushq $0
    pushq $33
    jmp loos_isr_common

.global loos_isr_34
loos_isr_34:
    pushq $0
    pushq $34
    jmp loos_isr_common

.global loos_isr_35
loos_isr_35:
    pushq $0
    pushq $35
    jmp loos_isr_common

.global loos_isr_36
loos_isr_36:
    pushq $0
    pushq $36
    jmp loos_isr_common

.global loos_isr_37
loos_isr_37:
    pushq $0
    pushq $37
    jmp loos_isr_common

.global loos_isr_38
loos_isr_38:
    pushq $0
    pushq $38
    jmp loos_isr_common

.global loos_isr_39
loos_isr_39:
    pushq $0
    pushq $39
    jmp loos_isr_common

.global loos_isr_40
loos_isr_40:
    pushq $0
    pushq $40
    jmp loos_isr_common

.global loos_isr_41
loos_isr_41:
    pushq $0
    pushq $41
    jmp loos_isr_common

.global loos_isr_42
loos_isr_42:
    pushq $0
    pushq $42
    jmp loos_isr_common

.global loos_isr_43
loos_isr_43:
    pushq $0
    pushq $43
    jmp loos_isr_common

.global loos_isr_44
loos_isr_44:
    pushq $0
    pushq $44
    jmp loos_isr_common

.global loos_isr_45
loos_isr_45:
    pushq $0
    pushq $45
    jmp loos_isr_common

.global loos_isr_46
loos_isr_46:
    pushq $0
    pushq $46
    jmp loos_isr_common

.global loos_isr_47
loos_isr_47:
    pushq $0
    pushq $47
    jmp loos_isr_common

.global loos_isr_48
loos_isr_48:
    pushq $0
    pushq $48
    jmp loos_isr_common

.global loos_isr_49
loos_isr_49:
    pushq $0
    pushq $49
    jmp loos_isr_common

.global loos_isr_50
loos_isr_50:
    pushq $0
    pushq $50
    jmp loos_isr_common

.global loos_isr_51
loos_isr_51:
    pushq $0
    pushq $51
    jmp loos_isr_common

.global loos_isr_52
loos_isr_52:
    pushq $0
    pushq $52
    jmp loos_isr_common

.global loos_isr_53
loos_isr_53:
    pushq $0
    pushq $53
    jmp loos_isr_common

.global loos_isr_54
loos_isr_54:
    pushq $0
    pushq $54
    jmp loos_isr_common

.global loos_isr_55
loos_isr_55:
    pushq $0
    pushq $55
    jmp loos_isr_common

.global loos_isr_56
loos_isr_56:
    pushq $0
    pushq $56
    jmp loos_isr_common

.global loos_isr_57
loos_isr_57:
    pushq $0
    pushq $57
    jmp loos_isr_common

.global loos_isr_58
loos_isr_58:
    pushq $0
    pushq $58
    jmp loos_isr_common

.global loos_isr_59
loos_isr_59:
    pushq $0
    pushq $59
    jmp loos_isr_common

.global loos_isr_60
loos_isr_60:
    pushq $0
    pushq $60
    jmp loos_isr_common

.global loos_isr_61
loos_isr_61:
    pushq $0
    pushq $61
    jmp loos_isr_common

.global loos_isr_62
loos_isr_62:
    pushq $0
    pushq $62
    jmp loos_isr_common

.global loos_isr_63
loos_isr_63:
    pushq $0
    pushq $63
    jmp loos_isr_common

.global loos_isr_64
loos_isr_64:
    pushq $0
    pushq $64
    jmp loos_isr_common

.global loos_isr_65
loos_isr_65:
    pushq $0
    pushq $65
    jmp loos_isr_common

.global loos_isr_66
loos_isr_66:
    pushq $0
    pushq $66
    jmp loos_isr_common

.global loos_isr_67
loos_isr_67:
    pushq $0
    pushq $67
    jmp loos_isr_common

.global loos_isr_68
loos_isr_68:
    pushq $0
    pushq $68
    jmp loos_isr_common

.global loos_isr_69
loos_isr_69:
    pushq $0
    pushq $69
    jmp loos_isr_common

.global loos_isr_70
loos_isr_70:
    pushq $0
    pushq $70
    jmp loos_isr_common

.global loos_isr_71
loos_isr_71:
    pushq $0
    pushq $71
    jmp loos_isr_common

.global loos_isr_72
loos_isr_72:
    pushq $0
    pushq $72
    jmp loos_isr_common

.global loos_isr_73
loos_isr_73:
    pushq $0
    pushq $73
    jmp loos_isr_common

.global loos_isr_74
loos_isr_74:
    pushq $0
    pushq $74
    jmp loos_isr_common

.global loos_isr_75
loos_isr_75:
    pushq $0
    pushq $75
    jmp loos_isr_common

.global loos_isr_76
loos_isr_76:
    pushq $0
    pushq $76
    jmp loos_isr_common

.global loos_isr_77
loos_isr_77:
    pushq $0
    pushq $77
    jmp loos_isr_common

.global loos_isr_78
loos_isr_78:
    pushq $0
    pushq $78
    jmp loos_isr_common

.global loos_isr_79
loos_isr_79:
    pushq $0
    pushq $79
    jmp loos_isr_common

.global loos_isr_80
loos_isr_80:
    pushq $0
    pushq $80
    jmp loos_isr_common

.global loos_isr_81
loos_isr_81:
    pushq $0
    pushq $81
    jmp loos_isr_common

.global loos_isr_82
loos_isr_82:
    pushq $0
    pushq $82
    jmp loos_isr_common

.global loos_isr_83
loos_isr_83:
    pushq $0
    pushq $83
    jmp loos_isr_common

.global loos_isr_84
loos_isr_84:
    pushq $0
    pushq $84
    jmp loos_isr_common

.global loos_isr_85
loos_isr_85:
    pushq $0
    pushq $85
    jmp loos_isr_common

.global loos_isr_86
loos_isr_86:
    pushq $0
    pushq $86
    jmp loos_isr_common

.global loos_isr_87
loos_isr_87:
    pushq $0
    pushq $87
    jmp loos_isr_common

.global loos_isr_88
loos_isr_88:
    pushq $0
    pushq $88
    jmp loos_isr_common

.global loos_isr_89
loos_isr_89:
    pushq $0
    pushq $89
    jmp loos_isr_common

.global loos_isr_90
loos_isr_90:
    pushq $0
    pushq $90
    jmp loos_isr_common

.global loos_isr_91
loos_isr_91:
    pushq $0
    pushq $91
    jmp loos_isr_common

.global loos_isr_92
loos_isr_92:
    pushq $0
    pushq $92
    jmp loos_isr_common

.global loos_isr_93
loos_isr_93:
    pushq $0
    pushq $93
    jmp loos_isr_common

.global loos_isr_94
loos_isr_94:
    pushq $0
    pushq $94
    jmp loos_isr_common

.global loos_isr_95
loos_isr_95:
    pushq $0
    pushq $95
    jmp loos_isr_common

.global loos_isr_96
loos_isr_96:
    pushq $0
    pushq $96
    jmp loos_isr_common

.global loos_isr_97
loos_isr_97:
    pushq $0
    pushq $97
    jmp loos_isr_common

.global loos_isr_98
loos_isr_98:
    pushq $0
    pushq $98
    jmp loos_isr_common

.global loos_isr_99
loos_isr_99:
    pushq $0
    pushq $99
    jmp loos_isr_common

.global loos_isr_100
loos_isr_100:
    pushq $0
    pushq $100
    jmp loos_isr_common

.global loos_isr_101
loos_isr_101:
    pushq $0
    pushq $101
    jmp loos_isr_common

.global loos_isr_102
loos_isr_102:
    pushq $0
    pushq $102
    jmp loos_isr_common

.global loos_isr_103
loos_isr_103:
    pushq $0
    pushq $103
    jmp loos_isr_common

.global loos_isr_104
loos_isr_104:
    pushq $0
    pushq $104
    jmp loos_isr_common

.global loos_isr_105
loos_isr_105:
    pushq $0
    pushq $105
    jmp loos_isr_common

.global loos_isr_106
loos_isr_106:
    pushq $0
    pushq $106
    jmp loos_isr_common

.global loos_isr_107
loos_isr_107:
    pushq $0
    pushq $107
    jmp loos_isr_common

.global loos_isr_108
loos_isr_108:
    pushq $0
    pushq $108
    jmp loos_isr_common

.global loos_isr_109
loos_isr_109:
    pushq $0
    pushq $109
    jmp loos_isr_common

.global loos_isr_110
loos_isr_110:
    pushq $0
    pushq $110
    jmp loos_isr_common

.global loos_isr_111
loos_isr_111:
    pushq $0
    pushq $111
    jmp loos_isr_common

.global loos_isr_112
loos_isr_112:
    pushq $0
    pushq $112
    jmp loos_isr_common

.global loos_isr_113
loos_isr_113:
    pushq $0
    pushq $113
    jmp loos_isr_common

.global loos_isr_114
loos_isr_114:
    pushq $0
    pushq $114
    jmp loos_isr_common

.global loos_isr_115
loos_isr_115:
    pushq $0
    pushq $115
    jmp loos_isr_common

.global loos_isr_116
loos_isr_116:
    pushq $0
    pushq $116
    jmp loos_isr_common

.global loos_isr_117
loos_isr_117:
    pushq $0
    pushq $117
    jmp loos_isr_common

.global loos_isr_118
loos_isr_118:
    pushq $0
    pushq $118
    jmp loos_isr_common

.global loos_isr_119
loos_isr_119:
    pushq $0
    pushq $119
    jmp loos_isr_common

.global loos_isr_120
loos_isr_120:
    pushq $0
    pushq $120
    jmp loos_isr_common

.global loos_isr_121
loos_isr_121:
    pushq $0
    pushq $121
    jmp loos_isr_common

.global loos_isr_122
loos_isr_122:
    pushq $0
    pushq $122
    jmp loos_isr_common

.global loos_isr_123
loos_isr_123:
    pushq $0
    pushq $123
    jmp loos_isr_common

.global loos_isr_124
loos_isr_124:
    pushq $0
    pushq $124
    jmp loos_isr_common

.global loos_isr_125
loos_isr_125:
    pushq $0
    pushq $125
    jmp loos_isr_common

.global loos_isr_126
loos_isr_126:
    pushq $0
    pushq $126
    jmp loos_isr_common

.global loos_isr_127
loos_isr_127:
    pushq $0
    pushq $127
    jmp loos_isr_common

.global loos_isr_128
loos_isr_128:
    pushq $0
    pushq $128
    jmp loos_isr_common

.global loos_isr_129
loos_isr_129:
    pushq $0
    pushq $129
    jmp loos_isr_common

.global loos_isr_130
loos_isr_130:
    pushq $0
    pushq $130
    jmp loos_isr_common

.global loos_isr_131
loos_isr_131:
    pushq $0
    pushq $131
    jmp loos_isr_common

.global loos_isr_132
loos_isr_132:
    pushq $0
    pushq $132
    jmp loos_isr_common

.global loos_isr_133
loos_isr_133:
    pushq $0
    pushq $133
    jmp loos_isr_common

.global loos_isr_134
loos_isr_134:
    pushq $0
    pushq $134
    jmp loos_isr_common

.global loos_isr_135
loos_isr_135:
    pushq $0
    pushq $135
    jmp loos_isr_common

.global loos_isr_136
loos_isr_136:
    pushq $0
    pushq $136
    jmp loos_isr_common

.global loos_isr_137
loos_isr_137:
    pushq $0
    pushq $137
    jmp loos_isr_common

.global loos_isr_138
loos_isr_138:
    pushq $0
    pushq $138
    jmp loos_isr_common

.global loos_isr_139
loos_isr_139:
    pushq $0
    pushq $139
    jmp loos_isr_common

.global loos_isr_140
loos_isr_140:
    pushq $0
    pushq $140
    jmp loos_isr_common

.global loos_isr_141
loos_isr_141:
    pushq $0
    pushq $141
    jmp loos_isr_common

.global loos_isr_142
loos_isr_142:
    pushq $0
    pushq $142
    jmp loos_isr_common

.global loos_isr_143
loos_isr_143:
    pushq $0
    pushq $143
    jmp loos_isr_common

.global loos_isr_144
loos_isr_144:
    pushq $0
    pushq $144
    jmp loos_isr_common

.global loos_isr_145
loos_isr_145:
    pushq $0
    pushq $145
    jmp loos_isr_common

.global loos_isr_146
loos_isr_146:
    pushq $0
    pushq $146
    jmp loos_isr_common

.global loos_isr_147
loos_isr_147:
    pushq $0
    pushq $147
    jmp loos_isr_common

.global loos_isr_148
loos_isr_148:
    pushq $0
    pushq $148
    jmp loos_isr_common

.global loos_isr_149
loos_isr_149:
    pushq $0
    pushq $149
    jmp loos_isr_common

.global loos_isr_150
loos_isr_150:
    pushq $0
    pushq $150
    jmp loos_isr_common

.global loos_isr_151
loos_isr_151:
    pushq $0
    pushq $151
    jmp loos_isr_common

.global loos_isr_152
loos_isr_152:
    pushq $0
    pushq $152
    jmp loos_isr_common

.global loos_isr_153
loos_isr_153:
    pushq $0
    pushq $153
    jmp loos_isr_common

.global loos_isr_154
loos_isr_154:
    pushq $0
    pushq $154
    jmp loos_isr_common

.global loos_isr_155
loos_isr_155:
    pushq $0
    pushq $155
    jmp loos_isr_common

.global loos_isr_156
loos_isr_156:
    pushq $0
    pushq $156
    jmp loos_isr_common

.global loos_isr_157
loos_isr_157:
    pushq $0
    pushq $157
    jmp loos_isr_common

.global loos_isr_158
loos_isr_158:
    pushq $0
    pushq $158
    jmp loos_isr_common

.global loos_isr_159
loos_isr_159:
    pushq $0
    pushq $159
    jmp loos_isr_common

.global loos_isr_160
loos_isr_160:
    pushq $0
    pushq $160
    jmp loos_isr_common

.global loos_isr_161
loos_isr_161:
    pushq $0
    pushq $161
    jmp loos_isr_common

.global loos_isr_162
loos_isr_162:
    pushq $0
    pushq $162
    jmp loos_isr_common

.global loos_isr_163
loos_isr_163:
    pushq $0
    pushq $163
    jmp loos_isr_common

.global loos_isr_164
loos_isr_164:
    pushq $0
    pushq $164
    jmp loos_isr_common

.global loos_isr_165
loos_isr_165:
    pushq $0
    pushq $165
    jmp loos_isr_common

.global loos_isr_166
loos_isr_166:
    pushq $0
    pushq $166
    jmp loos_isr_common

.global loos_isr_167
loos_isr_167:
    pushq $0
    pushq $167
    jmp loos_isr_common

.global loos_isr_168
loos_isr_168:
    pushq $0
    pushq $168
    jmp loos_isr_common

.global loos_isr_169
loos_isr_169:
    pushq $0
    pushq $169
    jmp loos_isr_common

.global loos_isr_170
loos_isr_170:
    pushq $0
    pushq $170
    jmp loos_isr_common

.global loos_isr_171
loos_isr_171:
    pushq $0
    pushq $171
    jmp loos_isr_common

.global loos_isr_172
loos_isr_172:
    pushq $0
    pushq $172
    jmp loos_isr_common

.global loos_isr_173
loos_isr_173:
    pushq $0
    pushq $173
    jmp loos_isr_common

.global loos_isr_174
loos_isr_174:
    pushq $0
    pushq $174
    jmp loos_isr_common

.global loos_isr_175
loos_isr_175:
    pushq $0
    pushq $175
    jmp loos_isr_common

.global loos_isr_176
loos_isr_176:
    pushq $0
    pushq $176
    jmp loos_isr_common

.global loos_isr_177
loos_isr_177:
    pushq $0
    pushq $177
    jmp loos_isr_common

.global loos_isr_178
loos_isr_178:
    pushq $0
    pushq $178
    jmp loos_isr_common

.global loos_isr_179
loos_isr_179:
    pushq $0
    pushq $179
    jmp loos_isr_common

.global loos_isr_180
loos_isr_180:
    pushq $0
    pushq $180
    jmp loos_isr_common

.global loos_isr_181
loos_isr_181:
    pushq $0
    pushq $181
    jmp loos_isr_common

.global loos_isr_182
loos_isr_182:
    pushq $0
    pushq $182
    jmp loos_isr_common

.global loos_isr_183
loos_isr_183:
    pushq $0
    pushq $183
    jmp loos_isr_common

.global loos_isr_184
loos_isr_184:
    pushq $0
    pushq $184
    jmp loos_isr_common

.global loos_isr_185
loos_isr_185:
    pushq $0
    pushq $185
    jmp loos_isr_common

.global loos_isr_186
loos_isr_186:
    pushq $0
    pushq $186
    jmp loos_isr_common

.global loos_isr_187
loos_isr_187:
    pushq $0
    pushq $187
    jmp loos_isr_common

.global loos_isr_188
loos_isr_188:
    pushq $0
    pushq $188
    jmp loos_isr_common

.global loos_isr_189
loos_isr_189:
    pushq $0
    pushq $189
    jmp loos_isr_common

.global loos_isr_190
loos_isr_190:
    pushq $0
    pushq $190
    jmp loos_isr_common

.global loos_isr_191
loos_isr_191:
    pushq $0
    pushq $191
    jmp loos_isr_common

.global loos_isr_192
loos_isr_192:
    pushq $0
    pushq $192
    jmp loos_isr_common

.global loos_isr_193
loos_isr_193:
    pushq $0
    pushq $193
    jmp loos_isr_common

.global loos_isr_194
loos_isr_194:
    pushq $0
    pushq $194
    jmp loos_isr_common

.global loos_isr_195
loos_isr_195:
    pushq $0
    pushq $195
    jmp loos_isr_common

.global loos_isr_196
loos_isr_196:
    pushq $0
    pushq $196
    jmp loos_isr_common

.global loos_isr_197
loos_isr_197:
    pushq $0
    pushq $197
    jmp loos_isr_common

.global loos_isr_198
loos_isr_198:
    pushq $0
    pushq $198
    jmp loos_isr_common

.global loos_isr_199
loos_isr_199:
    pushq $0
    pushq $199
    jmp loos_isr_common

.global loos_isr_200
loos_isr_200:
    pushq $0
    pushq $200
    jmp loos_isr_common

.global loos_isr_201
loos_isr_201:
    pushq $0
    pushq $201
    jmp loos_isr_common

.global loos_isr_202
loos_isr_202:
    pushq $0
    pushq $202
    jmp loos_isr_common

.global loos_isr_203
loos_isr_203:
    pushq $0
    pushq $203
    jmp loos_isr_common

.global loos_isr_204
loos_isr_204:
    pushq $0
    pushq $204
    jmp loos_isr_common

.global loos_isr_205
loos_isr_205:
    pushq $0
    pushq $205
    jmp loos_isr_common

.global loos_isr_206
loos_isr_206:
    pushq $0
    pushq $206
    jmp loos_isr_common

.global loos_isr_207
loos_isr_207:
    pushq $0
    pushq $207
    jmp loos_isr_common

.global loos_isr_208
loos_isr_208:
    pushq $0
    pushq $208
    jmp loos_isr_common

.global loos_isr_209
loos_isr_209:
    pushq $0
    pushq $209
    jmp loos_isr_common

.global loos_isr_210
loos_isr_210:
    pushq $0
    pushq $210
    jmp loos_isr_common

.global loos_isr_211
loos_isr_211:
    pushq $0
    pushq $211
    jmp loos_isr_common

.global loos_isr_212
loos_isr_212:
    pushq $0
    pushq $212
    jmp loos_isr_common

.global loos_isr_213
loos_isr_213:
    pushq $0
    pushq $213
    jmp loos_isr_common

.global loos_isr_214
loos_isr_214:
    pushq $0
    pushq $214
    jmp loos_isr_common

.global loos_isr_215
loos_isr_215:
    pushq $0
    pushq $215
    jmp loos_isr_common

.global loos_isr_216
loos_isr_216:
    pushq $0
    pushq $216
    jmp loos_isr_common

.global loos_isr_217
loos_isr_217:
    pushq $0
    pushq $217
    jmp loos_isr_common

.global loos_isr_218
loos_isr_218:
    pushq $0
    pushq $218
    jmp loos_isr_common

.global loos_isr_219
loos_isr_219:
    pushq $0
    pushq $219
    jmp loos_isr_common

.global loos_isr_220
loos_isr_220:
    pushq $0
    pushq $220
    jmp loos_isr_common

.global loos_isr_221
loos_isr_221:
    pushq $0
    pushq $221
    jmp loos_isr_common

.global loos_isr_222
loos_isr_222:
    pushq $0
    pushq $222
    jmp loos_isr_common

.global loos_isr_223
loos_isr_223:
    pushq $0
    pushq $223
    jmp loos_isr_common

.global loos_isr_224
loos_isr_224:
    pushq $0
    pushq $224
    jmp loos_isr_common

.global loos_isr_225
loos_isr_225:
    pushq $0
    pushq $225
    jmp loos_isr_common

.global loos_isr_226
loos_isr_226:
    pushq $0
    pushq $226
    jmp loos_isr_common

.global loos_isr_227
loos_isr_227:
    pushq $0
    pushq $227
    jmp loos_isr_common

.global loos_isr_228
loos_isr_228:
    pushq $0
    pushq $228
    jmp loos_isr_common

.global loos_isr_229
loos_isr_229:
    pushq $0
    pushq $229
    jmp loos_isr_common

.global loos_isr_230
loos_isr_230:
    pushq $0
    pushq $230
    jmp loos_isr_common

.global loos_isr_231
loos_isr_231:
    pushq $0
    pushq $231
    jmp loos_isr_common

.global loos_isr_232
loos_isr_232:
    pushq $0
    pushq $232
    jmp loos_isr_common

.global loos_isr_233
loos_isr_233:
    pushq $0
    pushq $233
    jmp loos_isr_common

.global loos_isr_234
loos_isr_234:
    pushq $0
    pushq $234
    jmp loos_isr_common

.global loos_isr_235
loos_isr_235:
    pushq $0
    pushq $235
    jmp loos_isr_common

.global loos_isr_236
loos_isr_236:
    pushq $0
    pushq $236
    jmp loos_isr_common

.global loos_isr_237
loos_isr_237:
    pushq $0
    pushq $237
    jmp loos_isr_common

.global loos_isr_238
loos_isr_238:
    pushq $0
    pushq $238
    jmp loos_isr_common

.global loos_isr_239
loos_isr_239:
    pushq $0
    pushq $239
    jmp loos_isr_common

.global loos_isr_240
loos_isr_240:
    pushq $0
    pushq $240
    jmp loos_isr_common

.global loos_isr_241
loos_isr_241:
    pushq $0
    pushq $241
    jmp loos_isr_common

.global loos_isr_242
loos_isr_242:
    pushq $0
    pushq $242
    jmp loos_isr_common

.global loos_isr_243
loos_isr_243:
    pushq $0
    pushq $243
    jmp loos_isr_common

.global loos_isr_244
loos_isr_244:
    pushq $0
    pushq $244
    jmp loos_isr_common

.global loos_isr_245
loos_isr_245:
    pushq $0
    pushq $245
    jmp loos_isr_common

.global loos_isr_246
loos_isr_246:
    pushq $0
    pushq $246
    jmp loos_isr_common

.global loos_isr_247
loos_isr_247:
    pushq $0
    pushq $247
    jmp loos_isr_common

.global loos_isr_248
loos_isr_248:
    pushq $0
    pushq $248
    jmp loos_isr_common

.global loos_isr_249
loos_isr_249:
    pushq $0
    pushq $249
    jmp loos_isr_common

.global loos_isr_250
loos_isr_250:
    pushq $0
    pushq $250
    jmp loos_isr_common

.global loos_isr_251
loos_isr_251:
    pushq $0
    pushq $251
    jmp loos_isr_common

.global loos_isr_252
loos_isr_252:
    pushq $0
    pushq $252
    jmp loos_isr_common

.global loos_isr_253
loos_isr_253:
    pushq $0
    pushq $253
    jmp loos_isr_common

.global loos_isr_254
loos_isr_254:
    pushq $0
    pushq $254
    jmp loos_isr_common

.global loos_isr_255
loos_isr_255:
    pushq $0
    pushq $255
    jmp loos_isr_common

.section .rodata
.global loos_isr_table
loos_isr_table:
    .quad loos_isr_0
    .quad loos_isr_1
    .quad loos_isr_2
    .quad loos_isr_3
    .quad loos_isr_4
    .quad loos_isr_5
    .quad loos_isr_6
    .quad loos_isr_7
    .quad loos_isr_8
    .quad loos_isr_9
    .quad loos_isr_10
    .quad loos_isr_11
    .quad loos_isr_12
    .quad loos_isr_13
    .quad loos_isr_14
    .quad loos_isr_15
    .quad loos_isr_16
    .quad loos_isr_17
    .quad loos_isr_18
    .quad loos_isr_19
    .quad loos_isr_20
    .quad loos_isr_21
    .quad loos_isr_22
    .quad loos_isr_23
    .quad loos_isr_24
    .quad loos_isr_25
    .quad loos_isr_26
    .quad loos_isr_27
    .quad loos_isr_28
    .quad loos_isr_29
    .quad loos_isr_30
    .quad loos_isr_31
    .quad loos_isr_32
    .quad loos_isr_33
    .quad loos_isr_34
    .quad loos_isr_35
    .quad loos_isr_36
    .quad loos_isr_37
    .quad loos_isr_38
    .quad loos_isr_39
    .quad loos_isr_40
    .quad loos_isr_41
    .quad loos_isr_42
    .quad loos_isr_43
    .quad loos_isr_44
    .quad loos_isr_45
    .quad loos_isr_46
    .quad loos_isr_47
    .quad loos_isr_48
    .quad loos_isr_49
    .quad loos_isr_50
    .quad loos_isr_51
    .quad loos_isr_52
    .quad loos_isr_53
    .quad loos_isr_54
    .quad loos_isr_55
    .quad loos_isr_56
    .quad loos_isr_57
    .quad loos_isr_58
    .quad loos_isr_59
    .quad loos_isr_60
    .quad loos_isr_61
    .quad loos_isr_62
    .quad loos_isr_63
    .quad loos_isr_64
    .quad loos_isr_65
    .quad loos_isr_66
    .quad loos_isr_67
    .quad loos_isr_68
    .quad loos_isr_69
    .quad loos_isr_70
    .quad loos_isr_71
    .quad loos_isr_72
    .quad loos_isr_73
    .quad loos_isr_74
    .quad loos_isr_75
    .quad loos_isr_76
    .quad loos_isr_77
    .quad loos_isr_78
    .quad loos_isr_79
    .quad loos_isr_80
    .quad loos_isr_81
    .quad loos_isr_82
    .quad loos_isr_83
    .quad loos_isr_84
    .quad loos_isr_85
    .quad loos_isr_86
    .quad loos_isr_87
    .quad loos_isr_88
    .quad loos_isr_89
    .quad loos_isr_90
    .quad loos_isr_91
    .quad loos_isr_92
    .quad loos_isr_93
    .quad loos_isr_94
    .quad loos_isr_95
    .quad loos_isr_96
    .quad loos_isr_97
    .quad loos_isr_98
    .quad loos_isr_99
    .quad loos_isr_100
    .quad loos_isr_101
    .quad loos_isr_102
    .quad loos_isr_103
    .quad loos_isr_104
    .quad loos_isr_105
    .quad loos_isr_106
    .quad loos_isr_107
    .quad loos_isr_108
    .quad loos_isr_109
    .quad loos_isr_110
    .quad loos_isr_111
    .quad loos_isr_112
    .quad loos_isr_113
    .quad loos_isr_114
    .quad loos_isr_115
    .quad loos_isr_116
    .quad loos_isr_117
    .quad loos_isr_118
    .quad loos_isr_119
    .quad loos_isr_120
    .quad loos_isr_121
    .quad loos_isr_122
    .quad loos_isr_123
    .quad loos_isr_124
    .quad loos_isr_125
    .quad loos_isr_126
    .quad loos_isr_127
    .quad loos_isr_128
    .quad loos_isr_129
    .quad loos_isr_130
    .quad loos_isr_131
    .quad loos_isr_132
    .quad loos_isr_133
    .quad loos_isr_134
    .quad loos_isr_135
    .quad loos_isr_136
    .quad loos_isr_137
    .quad loos_isr_138
    .quad loos_isr_139
    .quad loos_isr_140
    .quad loos_isr_141
    .quad loos_isr_142
    .quad loos_isr_143
    .quad loos_isr_144
    .quad loos_isr_145
    .quad loos_isr_146
    .quad loos_isr_147
    .quad loos_isr_148
    .quad loos_isr_149
    .quad loos_isr_150
    .quad loos_isr_151
    .quad loos_isr_152
    .quad loos_isr_153
    .quad loos_isr_154
    .quad loos_isr_155
    .quad loos_isr_156
    .quad loos_isr_157
    .quad loos_isr_158
    .quad loos_isr_159
    .quad loos_isr_160
    .quad loos_isr_161
    .quad loos_isr_162
    .quad loos_isr_163
    .quad loos_isr_164
    .quad loos_isr_165
    .quad loos_isr_166
    .quad loos_isr_167
    .quad loos_isr_168
    .quad loos_isr_169
    .quad loos_isr_170
    .quad loos_isr_171
    .quad loos_isr_172
    .quad loos_isr_173
    .quad loos_isr_174
    .quad loos_isr_175
    .quad loos_isr_176
    .quad loos_isr_177
    .quad loos_isr_178
    .quad loos_isr_179
    .quad loos_isr_180
    .quad loos_isr_181
    .quad loos_isr_182
    .quad loos_isr_183
    .quad loos_isr_184
    .quad loos_isr_185
    .quad loos_isr_186
    .quad loos_isr_187
    .quad loos_isr_188
    .quad loos_isr_189
    .quad loos_isr_190
    .quad loos_isr_191
    .quad loos_isr_192
    .quad loos_isr_193
    .quad loos_isr_194
    .quad loos_isr_195
    .quad loos_isr_196
    .quad loos_isr_197
    .quad loos_isr_198
    .quad loos_isr_199
    .quad loos_isr_200
    .quad loos_isr_201
    .quad loos_isr_202
    .quad loos_isr_203
    .quad loos_isr_204
    .quad loos_isr_205
    .quad loos_isr_206
    .quad loos_isr_207
    .quad loos_isr_208
    .quad loos_isr_209
    .quad loos_isr_210
    .quad loos_isr_211
    .quad loos_isr_212
    .quad loos_isr_213
    .quad loos_isr_214
    .quad loos_isr_215
    .quad loos_isr_216
    .quad loos_isr_217
    .quad loos_isr_218
    .quad loos_isr_219
    .quad loos_isr_220
    .quad loos_isr_221
    .quad loos_isr_222
    .quad loos_isr_223
    .quad loos_isr_224
    .quad loos_isr_225
    .quad loos_isr_226
    .quad loos_isr_227
    .quad loos_isr_228
    .quad loos_isr_229
    .quad loos_isr_230
    .quad loos_isr_231
    .quad loos_isr_232
    .quad loos_isr_233
    .quad loos_isr_234
    .quad loos_isr_235
    .quad loos_isr_236
    .quad loos_isr_237
    .quad loos_isr_238
    .quad loos_isr_239
    .quad loos_isr_240
    .quad loos_isr_241
    .quad loos_isr_242
    .quad loos_isr_243
    .quad loos_isr_244
    .quad loos_isr_245
    .quad loos_isr_246
    .quad loos_isr_247
    .quad loos_isr_248
    .quad loos_isr_249
    .quad loos_isr_250
    .quad loos_isr_251
    .quad loos_isr_252
    .quad loos_isr_253
    .quad loos_isr_254
    .quad loos_isr_255
