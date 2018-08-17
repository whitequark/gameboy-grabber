#include <string.h>
#include <stdio.h>
#include <sim_avr.h>
#include <sim_elf.h>
#include <sim_vcd_file.h>
#include <avr_ioport.h>
#include <avr_adc.h>

static void adc_trig_notify(struct avr_irq_t *irq, uint32_t value, void *param) {
  avr_t *avr = (avr_t *)param;
  static int adc_value;
  adc_value += 15;
  avr_raise_irq(avr_io_getirq(avr, AVR_IOCTL_ADC_GETIRQ, ADC_IRQ_ADC2), adc_value % 3300);
}

int main(int argc, char *argv[])
{
  elf_firmware_t fw;
  elf_read_firmware("firmware.elf", &fw);
  fw.frequency = 9600000;
  strcpy(fw.mmcu, "attiny13a");

  avr_t *avr = avr_make_mcu_by_name(fw.mmcu);
  avr_init(avr);
  avr_load_firmware(avr, &fw);

  avr->log = LOG_DEBUG;
  avr->vcc = 3300;

  avr_vcd_t vcd_file;
  avr_vcd_init(avr, "trace.vcd", &vcd_file, 1 /* usec */);
  avr_vcd_add_signal(&vcd_file, avr_io_getirq(avr, AVR_IOCTL_IOPORT_GETIRQ('B'), 3), 1, "PB3");
  avr_vcd_start(&vcd_file);

  avr_irq_register_notify(avr_io_getirq(avr, AVR_IOCTL_ADC_GETIRQ, ADC_IRQ_OUT_TRIGGER),
                          adc_trig_notify, avr);

  int state = cpu_Running;
  unsigned time = 0;
  while (state != cpu_Done && state != cpu_Crashed && time < 100000) {
    state = avr_run(avr);
    time++;
  }
}
