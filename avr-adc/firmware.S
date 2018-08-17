#include <avr/io.h>
#define SIMULATION
#define MOCK

    ; CPU frequency is approx 9.6 MHz

.text
    rjmp  reset
    nop   ; INT0
    nop   ; PCINT0
    nop   ; TIM0_OVF
    nop   ; EE_RDY
    nop   ; ANA_COMP
    rjmp  isr_tim0_compa
    nop   ; TIM0_COMPB
    nop   ; WDT
    rjmp  isr_adc

reset:
    ; set up stack
    ldi   r16, lo8(RAMEND)
    out   _SFR_IO_ADDR(SPL), r16

    ; configure sleep for ADC noise reduction mode
    ldi   r16, _BV(SE)|_BV(SM0)
    out   _SFR_IO_ADDR(MCUCR), r16

    ; enable TIM0 match A interrupt
    ldi   r16, _BV(OCIE0A)
    out   _SFR_IO_ADDR(TIMSK0), r16
    ; configure TIM0 for 1/256 prescaler, CTC mode, TOP value 3
    ; this is 9375 Hz or each 106 uS
    ldi   r16, 3
    out   _SFR_IO_ADDR(OCR0A), r16
    ldi   r16, _BV(WGM01)
    out   _SFR_IO_ADDR(TCCR0A), r16
    ldi   r16, _BV(CS02)
    out   _SFR_IO_ADDR(TCCR0B), r16

    ; disable channel 2 digital input buffer
    ldi   r16, _BV(ADC2D)
    out   _SFR_IO_ADDR(DIDR0), r16
    ; configure ADC for Vcc reference, channel 2 (PB4)
    ldi   r16, _BV(MUX1)
    out   _SFR_IO_ADDR(ADMUX), r16
    ; configure ADC prescaler for 1/64
    ; this is 150 kHz or 86 uS per conversion
    ; enable ADC and ADC interrupt
    ldi   r16, _BV(ADEN)|_BV(ADIE)|_BV(ADPS2)|_BV(ADPS1)
    out   _SFR_IO_ADDR(ADCSRA), r16

    ; configure PB3 as output
    ldi   r16, _BV(3)
    out   _SFR_IO_ADDR(PORTB), r16
    out   _SFR_IO_ADDR(DDRB), r16

    sei
1:  rjmp  1b

isr_tim0_compa:
#if defined(SIMULATION)
    ; simavr can't handle ADC noise reduction mode
    sbi   _SFR_IO_ADDR(ADCSRA), ADSC
#else
    sleep ; start conversion
#endif
    reti

isr_adc:
    ; transmitting data using a modified Manchester encoding:
    ; start is 00, 1 is 10, 0 is 01
    ; this encoding is self-synchronizing
    ;
    ; each instruction is approx 0.1 uS
    ; each bit time is approx 0.83 uS or 1.2 Mbaud exactly.
    ; 1+10 bits are transmitted in 9.12 uS
    ;
    ; the implementation is, of course, pipelined

#define DPIN 3

    ; start condition
    cbi   _SFR_IO_ADDR(PORTB), DPIN ; 2c
#if defined(MOCK)
    adiw  r24, 1                    ; 2c
#else
    in    r24, _SFR_IO_ADDR(ADCL)   ; 1c
    in    r25, _SFR_IO_ADDR(ADCH)   ; 1c
#endif
    bst   r25, 1                    ; 1c
    bld   r16, DPIN                 ; 1c
    nop                             ; 1c
    nop                             ; 1c

    ; data bit
.macro DATA rN b
    out   _SFR_IO_ADDR(PORTB), r16  ; 1c
    bst   \rN, \b                   ; 1c
    bld   r16, DPIN                 ; 1c
    nop                             ; 1c
    sbi   _SFR_IO_ADDR(PINB), DPIN  ; 2c
    nop                             ; 1c
    nop                             ; 1c
.endm

    ; 10-bit data word
    DATA r25 0
    DATA r24 7
    DATA r24 6
    DATA r24 5
    DATA r24 4
    DATA r24 3
    DATA r24 2
    DATA r24 1
    DATA r24 0
    DATA  r0 0 ; doesn't matter

    ; finally, stop condition
    sbi   _SFR_IO_ADDR(PORTB), DPIN ; 2c

    reti
