#pragma once

#include <string>
#include <vector>
#include <memory>
#include "point.h"

namespace geo {

enum class Kind { Circle, Rect };

/// NOTE: shapes are immutable once built.
class Shape {
public:
    explicit Shape(Kind kind);
    virtual ~Shape() = default;
    virtual double area() const = 0;
    std::string name() const;
    Kind kind() const { return kind_; }

protected:
    Kind kind_;
};

class Circle : public Shape {
public:
    explicit Circle(double r);
    double area() const override;

private:
    double r_;
    Point center_;
};

class Rect final : public Shape, private Named {
public:
    Rect(double w, double h);
    double area() const override;
    static Rect square(double side);

private:
    double w_, h_;
};

template <typename T>
class Registry {
public:
    void add(std::unique_ptr<T> item) { items_.push_back(std::move(item)); }
    std::size_t size() const { return items_.size(); }
    T *first() const;

private:
    std::vector<std::unique_ptr<T>> items_;
};

template <typename T>
T *Registry<T>::first() const {
    return items_.empty() ? nullptr : items_.front().get();
}

double total_area(const Registry<Shape> &reg);

namespace detail {
int round_up(double v);
}  // namespace detail

}  // namespace geo
