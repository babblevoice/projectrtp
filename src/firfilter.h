

#ifndef PROJECTRTPFIRFILTER_H
#define PROJECTRTPFIRFILTER_H

#include <stdint.h>

/*!md
# lowpass3_4k16k
Fixed FIR filter. everything fixed for speed.
16K sampling, 3.4K low pass.
*/
#define lowpass3_4k16kfl 17
class lowpass3_4k16k
{
public:
  lowpass3_4k16k();
  void reset( void );
  int16_t execute( int16_t val );

private:
  u_char round;
  float history[ lowpass3_4k16kfl ];
};


void testlofir( int frequency );


#endif /* PROJECTRTPFIRFILTER_H */

