
#ifndef PROJECTRTPNODEMAIN_H
#define PROJECTRTPNODEMAIN_H


typedef boost::asio::basic_waitable_timer< std::chrono::high_resolution_clock > ourhighrestimer;

namespace
boost {
  void throw_exception( std::exception const & e );
}

#endif /* PROJECTRTPNODEMAIN_H */
