use core::ptr;

use py32_hal::interrupt;

const PWR_BASE: u32 = 0x4000_7000;
const PWR_CR1: *mut u32 = PWR_BASE as *mut u32;
const PWR_CR1_LPR: u32 = 1 << 14;
const PWR_CR1_VOS_1V0: u32 = 1 << 9;
const PWR_CR1_SRAM_RETV_MASK: u32 = 0b111 << 16;
const PWR_CR1_SRAM_RETV_LOWEST: u32 = 0b011 << 16;

const SCB_SCR: *mut u32 = 0xE000_ED10 as *mut u32;
const SCB_SCR_SLEEPDEEP: u32 = 1 << 2;

fn configure_button_exti() {
    cortex_m::interrupt::free(|_| {
        let exti = py32_hal::pac::EXTI;

        exti.exticr(0).modify(|w| w.set_exti(0, 1)); // PB0 -> EXTI0
        exti.rtsr().modify(|w| w.set_line(0, false));
        exti.ftsr().modify(|w| w.set_line(0, true));
        exti.pr().write(|w| w.set_line(0, true));
        exti.imr().modify(|w| w.set_line(0, true));

        unsafe {
            cortex_m::peripheral::NVIC::unmask(interrupt::EXTI0_1);
        }
    });
}

fn clear_button_irq() {
    py32_hal::pac::EXTI.pr().write(|w| w.set_line(0, true));
    cortex_m::peripheral::NVIC::unpend(interrupt::EXTI0_1);
}

pub fn enter_stop_mode() {
    clear_button_irq();
    configure_button_exti();

    cortex_m::interrupt::free(|_| unsafe {
        py32_hal::pac::RCC.apbenr1().modify(|w| w.set_pwren(true));

        let mut pwr_cr1 = ptr::read_volatile(PWR_CR1);
        pwr_cr1 &= !PWR_CR1_SRAM_RETV_MASK;
        pwr_cr1 |= PWR_CR1_LPR | PWR_CR1_VOS_1V0 | PWR_CR1_SRAM_RETV_LOWEST;
        ptr::write_volatile(PWR_CR1, pwr_cr1);

        ptr::write_volatile(SCB_SCR, ptr::read_volatile(SCB_SCR) | SCB_SCR_SLEEPDEEP);
    });

    cortex_m::asm::dsb();
    cortex_m::asm::wfi();
    cortex_m::asm::isb();

    unsafe {
        ptr::write_volatile(SCB_SCR, ptr::read_volatile(SCB_SCR) & !SCB_SCR_SLEEPDEEP);
    }

    clear_button_irq();
}
