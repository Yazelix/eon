/* Force-included into the bundle's ncurses build so it runs on glibc older than 2.42.
 * glibc 2.42 made speed_t numeric (B9600 is 9600) and versioned cfgetospeed anew to
 * return numbers. This decodes the kernel's speed bits, which tcgetattr fills on any
 * glibc, into the same numbers, so ncurses keeps its numeric speed table and needs no
 * GLIBC_2.42 symbol. */
#include <termios.h>

static inline speed_t eon_cfgetospeed(const struct termios *termios)
{
    switch (termios->c_cflag & CBAUD) {
#define EON_SPEED(n) case __B##n: return B##n;
    EON_SPEED(0) EON_SPEED(50) EON_SPEED(75) EON_SPEED(110) EON_SPEED(134)
    EON_SPEED(150) EON_SPEED(200) EON_SPEED(300) EON_SPEED(600) EON_SPEED(1200)
    EON_SPEED(1800) EON_SPEED(2400) EON_SPEED(4800) EON_SPEED(9600) EON_SPEED(19200)
    EON_SPEED(38400) EON_SPEED(57600) EON_SPEED(115200) EON_SPEED(230400)
    EON_SPEED(460800) EON_SPEED(500000) EON_SPEED(576000) EON_SPEED(921600)
    EON_SPEED(1000000) EON_SPEED(1152000) EON_SPEED(1500000) EON_SPEED(2000000)
    EON_SPEED(2500000) EON_SPEED(3000000) EON_SPEED(3500000) EON_SPEED(4000000)
#undef EON_SPEED
    default:
        /* BOTHER: glibc 2.42 and later store the rate here. */
        return termios->c_ospeed;
    }
}
#define cfgetospeed eon_cfgetospeed
