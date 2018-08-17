firmware.ihex firmware.elf: firmware.S
	avr-gcc -nostdlib -g -mmcu=attiny13a firmware.S -o firmware.elf
	avr-objcopy -O ihex firmware.elf firmware.ihex

simulator: simulator.c
	$(CC) -o $@ $< `pkg-config --cflags --libs simavr`
