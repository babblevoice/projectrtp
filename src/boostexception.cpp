
#ifdef BOOST_NO_EXCEPTIONS

#include <boost/throw_exception.hpp>
#include <stdexcept>
#include <iostream>

/*
  Provided exception handlers when BOOST_NO_EXCEPTIONS is defined 
*/

namespace boost {
  void throw_exception( const std::exception& e ) {
    std::cerr << "Boost exception: " << e.what() << std::endl;
    std::terminate();
  }

  void throw_exception( const std::exception& e, const boost::source_location& ) {
    std::cerr << "Boost exception with location: " << e.what() << std::endl;
    std::terminate();
  }
}

#endif
