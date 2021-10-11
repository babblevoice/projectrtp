
#include <iostream>

#include <boost/asio.hpp>
#include "projectrtpbuffer.h"
#include "projectrtpfirfilter.h"

/*
My main driver in using this test suite is to test for buffer overruns and memory leaks.
Compiled with -fsanitize=address -fsanitize=leak
*/

boost::asio::io_context workercontext;

int main( void ) {

  try {
    testlowpass();
    testma();

    testrtpbuffer();

  } catch( const char *msg ) {
    std::cout << "ERROR: something didn't pass it's test please review" << std::endl;
    std::cout << msg << std::endl;
    return -1;
  }

  std::cout << "All tests passed" << std::endl;
  return 0;
}
